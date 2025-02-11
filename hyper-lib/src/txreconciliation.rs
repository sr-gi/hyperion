pub type ShortID = u32;

/// This is a hack. A sketch is really built using Minisketch. However, this is not necessary for the simulator.
/// The only thing we need to know is whether the node knows the transaction, and what is the size of the difference
/// between that and the set of transaction its peer knows (1 or 0, given we are simulating a single transactions).
/// The difference can be computed on the fly, but it is stored here so we can keep
/// track of the size of the message for statistics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Sketch {
    tx_set: bool,
    d: usize,
}

impl Sketch {
    pub fn new(tx_set: bool, d: usize) -> Self {
        Self { tx_set, d }
    }

    pub fn get_tx_set(&self) -> bool {
        self.tx_set
    }

    pub fn get_size(&self) -> usize {
        self.d
    }
}

#[derive(Clone)]
pub struct TxReconciliationState {
    /// Whether this peer is the reconciliation initiator or we are
    is_initiator: bool,
    /// Whether we are currently reconciling with this peer or not
    is_reconciling: bool,
    /// Whether the simulated transaction is in the reconciliation set
    recon_set: bool,
    /// The last Sketch we sent our peer if reconciling
    sketch: Option<Sketch>,
    /// Whether the simulated transaction is in pending to be added to the reconciliation set the next trickle.
    /// These is still unrequestable for privacy reasons (to prevent transaction proving), the transaction will became
    /// available once it would have been announced via fanout (on the next trickle).
    delayed_set: bool,
}

impl TxReconciliationState {
    pub fn new(is_initiator: bool) -> Self {
        Self {
            is_initiator,
            is_reconciling: false,
            recon_set: false,
            sketch: None,
            delayed_set: false,
        }
    }

    pub fn reset(&mut self) {
        self.clear_reconciling();
        self.delayed_set = false;
    }

    pub fn is_initiator(&self) -> bool {
        self.is_initiator
    }

    pub fn add_tx(&mut self) -> bool {
        let r = !self.delayed_set;
        self.delayed_set = true;

        r
    }

    /// Removes the transaction from the reconciliation set. This may happen if a peer has announced the transaction that we
    /// were planing to reconcile with them. Notice that, if this happens after creating a snapshot, the reconciliation will
    /// result in one additional INV (belonging to this transaction). This is equivalent to two INVs crossing, and AFAIK,
    /// there's nothing we can do about it
    pub fn remove_tx(&mut self) {
        assert!(!(self.recon_set && self.delayed_set));
        self.delayed_set = false;
        self.recon_set = false;
    }

    // Make delayed transactions available for reconciliation
    pub fn make_delayed_available(&mut self) {
        if self.delayed_set {
            self.recon_set = self.delayed_set;
            self.delayed_set = false;
        }
    }

    pub fn set_reconciling(&mut self) {
        self.is_reconciling = true;
    }

    pub fn clear_reconciling(&mut self) {
        // After reconciling we can safely clear the current recon_set.
        // If they wanted the simulated transaction, the reconciliation flow should have triggered
        // an announcement, and if they didn't want it, we don't need to offer it again
        // The delayed set will be cleared if an announcement is received
        self.recon_set = false;
        self.is_reconciling = false;
        self.sketch = None;
    }

    pub fn is_reconciling(&self) -> bool {
        self.is_reconciling
    }

    pub fn get_recon_set(&self) -> bool {
        self.recon_set
    }

    pub fn get_delayed_set(&self) -> bool {
        self.delayed_set
    }

    pub fn compute_sketch(&mut self, they_know_tx: bool) -> Sketch {
        // q cannot be easily predicted in a short simulation, however it is needed to size the sketch properly.
        // As a workaround, the sketches exchanges by the simulator are not really sketches, but knowledge of whether
        // the sender knows the given transaction. q can be scaled down if needed to mimic scenarios where the sketch
        // exchange is not perfectly efficient
        let local_set = self.get_recon_set();
        let remote_set = they_know_tx;
        // We can compute the size of the diff as int(A XOR B)
        // TODO: Scale q if required so the predicted difference is not always 100% accurate
        let q = (local_set ^ remote_set) as usize;
        let sketch = Sketch::new(local_set, q);
        self.sketch = Some(sketch.clone());

        sketch
    }

    pub fn get_last_sketch(&self) -> &Option<Sketch> {
        &self.sketch
    }

    pub fn compute_sketch_diff(&self, sketch: Sketch) -> (bool, bool) {
        let local_set = self.get_recon_set();
        let remote_set = sketch.get_tx_set();

        // A XOR B to see if there is a diff
        let diff = local_set ^ remote_set;
        // If there is a diff, we can compute whether to offer or request the transaction based on the other side's knowledge
        // No diff means that either both have it or none does, so there's nothing to send/request
        let request_tx = diff && remote_set;
        let offer_tx = diff && local_set;

        (request_tx, offer_tx)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_recon_state() {
        let mut tx_recon_state = TxReconciliationState::new(true);
        tx_recon_state.set_reconciling();
        assert!(tx_recon_state.is_reconciling());
        assert!(!tx_recon_state.recon_set);
        assert!(!tx_recon_state.delayed_set);

        // Add a transaction to the recon_set
        tx_recon_state.add_tx();

        // Check that the transaction has been added to the delayed
        // set, but the recon set remains empty
        assert!(!tx_recon_state.recon_set);
        assert!(tx_recon_state.delayed_set);
        assert!(tx_recon_state.is_reconciling());

        // Move to available and check again
        tx_recon_state.make_delayed_available();
        assert!(tx_recon_state.recon_set);
        assert!(!tx_recon_state.delayed_set);
        assert!(tx_recon_state.is_reconciling());

        // Clear, not including delayed (they are only included when cleaning after a simulation)
        // and check that both sets are empty
        tx_recon_state.clear_reconciling();
        assert!(!tx_recon_state.recon_set);
        assert!(!tx_recon_state.delayed_set);
        assert!(!tx_recon_state.is_reconciling());

        // Add again, leave data in delayed and clear
        tx_recon_state.add_tx();
        tx_recon_state.clear_reconciling();
        tx_recon_state.remove_tx();
        assert!(!tx_recon_state.recon_set);
        assert!(!tx_recon_state.delayed_set);
        assert!(!tx_recon_state.is_reconciling());

        // If data is held in recon_set, delayed_set must be empty
        // so not testing that case
    }

    #[test]
    fn test_sketch() {
        // Start from an empty state
        let mut tx_recon_state = TxReconciliationState::new(true);

        // Create their sketch without the transaction. Since none of us know the transaction, the diff size will be 0
        let mut diff_size = 0;
        let mut their_sketch = Sketch::new(false, diff_size);
        assert!(their_sketch.get_size() == diff_size);

        // Compute the diffs and check. None of us know the transaction so the diff should be false
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(!our_diff);
        assert!(!their_diff);

        // Add the tx to the recon set
        tx_recon_state.add_tx();
        tx_recon_state.make_delayed_available();

        // Change their sketch, since now the difference will be 1
        diff_size = 1;
        their_sketch = Sketch::new(false, diff_size);
        assert!(their_sketch.get_size() == diff_size);

        // Compute the diffs and check. We know the tx and they don't, so our diff should be false and theirs should be true
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(!our_diff);
        assert!(their_diff);

        // Update it so now we don't know but they do
        tx_recon_state.clear_reconciling();
        tx_recon_state.remove_tx();
        their_sketch = Sketch::new(true, diff_size);
        assert!(their_sketch.get_size() == diff_size);

        // Compute the diffs and check. They know the transaction and we don't, so out diff should be true and theirs should be false
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(our_diff);
        assert!(!their_diff);

        // Update it so both of us know the transaction
        diff_size = 0;
        tx_recon_state.add_tx();
        tx_recon_state.make_delayed_available();
        their_sketch = Sketch::new(true, diff_size);

        // Compute the diffs and check. We both know the transaction, so both diff should be false
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(!our_diff);
        assert!(!their_diff);
    }
}
