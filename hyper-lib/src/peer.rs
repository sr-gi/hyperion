use crate::txreconciliation::TxReconciliationState;

#[derive(Clone, PartialEq)]
pub(crate) enum TxAnnouncement {
    // We sent the announcement
    Sent,
    // We received the announcement
    Received,
    // The announcement is scheduled. [is_stale] flags whether this scheduling has become
    // stale (by having requested the relevant transaction via reconciliation already)
    Scheduled(/*is_stale*/ bool),
    // No announcement has been exchanged
    None,
}

/// A minimal abstraction of a peer
#[derive(Clone)]
pub struct Peer {
    /// Whether this peer started the connection, or we did
    is_inbounds: bool,
    /// Whether the simulated transaction has been announced to/by this peer
    tx_announcement: TxAnnouncement,
    /// Transaction reconciliation related data for peers that support Erlay
    tx_reconciliation_state: Option<TxReconciliationState>,
}

impl Peer {
    pub(crate) fn new(is_erlay: bool, is_inbounds: bool) -> Self {
        let tx_reconciliation_state = if is_erlay {
            // Connection initiators match reconciliation initiators
            // https://github.com/bitcoin/bips/blob/master/bip-0330.mediawiki#sendtxrcncl
            Some(TxReconciliationState::new(is_inbounds))
        } else {
            None
        };
        Self {
            is_inbounds,
            tx_announcement: TxAnnouncement::None,
            tx_reconciliation_state,
        }
    }

    /// Reset the peer state so a new round of the simulation can be run from a clean state
    pub fn reset(&mut self) {
        self.tx_announcement = TxAnnouncement::None;
        if let Some(recon_state) = self.get_tx_reconciliation_state_mut() {
            recon_state.reset();
        }
    }

    pub fn they_announced_tx(&self) -> bool {
        matches!(self.tx_announcement, TxAnnouncement::Received)
    }

    pub fn we_announced_tx(&self) -> bool {
        matches!(self.tx_announcement, TxAnnouncement::Sent)
    }

    pub fn already_announced(&self) -> bool {
        self.they_announced_tx() || self.we_announced_tx()
    }

    pub(crate) fn to_be_announced(&self) -> bool {
        matches!(self.tx_announcement, TxAnnouncement::Scheduled(false))
    }

    pub(crate) fn add_tx_announcement(&mut self, tx_announcement: TxAnnouncement) {
        assert!(!self.already_announced());
        self.tx_announcement = tx_announcement;
        if let Some(recon_state) = self.get_tx_reconciliation_state_mut() {
            recon_state.remove_tx();
        }
    }

    pub(crate) fn schedule_tx_announcement(&mut self) {
        assert!(matches!(self.tx_announcement, TxAnnouncement::None));
        self.tx_announcement = TxAnnouncement::Scheduled(false);
    }

    pub(crate) fn flag_as_stale(&mut self) {
        assert!(matches!(
            self.tx_announcement,
            TxAnnouncement::Scheduled(false)
        ));
        self.tx_announcement = TxAnnouncement::Scheduled(true);
    }

    pub(crate) fn is_erlay(&self) -> bool {
        self.tx_reconciliation_state.is_some()
    }

    pub(crate) fn is_inbounds(&self) -> bool {
        self.is_inbounds
    }

    pub(crate) fn add_tx_to_reconcile(&mut self) -> bool {
        self.tx_reconciliation_state
            .as_mut()
            .map(|recon_set| recon_set.add_tx())
            .unwrap_or(false)
    }

    pub(crate) fn get_tx_reconciliation_state(&self) -> Option<&TxReconciliationState> {
        self.tx_reconciliation_state.as_ref()
    }

    pub(crate) fn get_tx_reconciliation_state_mut(&mut self) -> Option<&mut TxReconciliationState> {
        self.tx_reconciliation_state.as_mut()
    }
}

#[cfg(test)]
mod test_peer {
    use super::*;

    #[test]
    fn test_new() {
        // Inbounds are transaction reconciliation initiators
        let inbound_erlay_peer = Peer::new(true, true);
        assert!(inbound_erlay_peer.is_erlay());
        assert!(
            inbound_erlay_peer.tx_reconciliation_state.is_some()
                && inbound_erlay_peer
                    .tx_reconciliation_state
                    .unwrap()
                    .is_initiator()
        );

        let outbound_erlay_peer = Peer::new(/*is_erlay=*/ true, /*is_inbound=*/ false);
        assert!(outbound_erlay_peer.is_erlay());
        assert!(
            outbound_erlay_peer.tx_reconciliation_state.is_some()
                && !outbound_erlay_peer
                    .tx_reconciliation_state
                    .unwrap()
                    .is_initiator()
        );

        // is_inbound is irrelevant here
        let fanout_peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        assert!(!fanout_peer.is_erlay());
        assert!(fanout_peer.tx_reconciliation_state.is_none());
    }

    #[test]
    fn test_add_announced_tx() {
        // Not exchanged
        let mut fanout_peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        assert!(!fanout_peer.already_announced());
        // Sent
        fanout_peer.add_tx_announcement(TxAnnouncement::Sent);
        assert!(fanout_peer.already_announced());
        assert!(fanout_peer.we_announced_tx());

        // Received
        fanout_peer.reset();
        fanout_peer.add_tx_announcement(TxAnnouncement::Received);
        assert!(fanout_peer.already_announced());
        assert!(fanout_peer.they_announced_tx());

        // The same applied for Erlay
        let mut erlay_peer = Peer::new(/*is_erlay=*/ true, /*is_inbound=*/ false);
        assert!(!erlay_peer.already_announced());
        erlay_peer.add_tx_announcement(TxAnnouncement::Sent);
        assert!(erlay_peer.already_announced());
        assert!(erlay_peer.we_announced_tx());

        erlay_peer.reset();
        assert!(!erlay_peer.already_announced());
        erlay_peer.add_tx_announcement(TxAnnouncement::Received);
        assert!(erlay_peer.already_announced());
        assert!(erlay_peer.they_announced_tx());

        // If an erlay peer has a transaction on their pending to be reconciled (either delayed or on the set)
        // and we add it to announced, it will be removed from reconciliation
        erlay_peer.reset();
        erlay_peer.add_tx_to_reconcile();

        // The transaction is in the reconciliation set before flagging it as known
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());

        // And is removed after
        erlay_peer.add_tx_announcement(TxAnnouncement::Sent);
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
        assert!(erlay_peer.already_announced());
    }

    #[test]
    fn test_add_tx_to_reconcile() {
        // Fanout peers have no reconciliation state, so data cannot be added to it
        let mut fanout_peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        assert!(!fanout_peer.add_tx_to_reconcile());

        // Erlay peers do have reconciliation state, independently of whether they are initiators or not
        let mut erlay_peer = Peer::new(/*is_erlay=*/ true, /*is_inbound=*/ false);
        assert!(erlay_peer.add_tx_to_reconcile());
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
    }
}
