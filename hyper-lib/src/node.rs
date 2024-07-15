use std::collections::{hash_map::Entry, HashMap, HashSet};

use rand::rngs::StdRng;

use rand_distr::{Distribution, Exp};

use crate::network::NetworkMessage;
use crate::simulator::Event;
use crate::{TxId, SECS_TO_NANOS};

pub type NodeId = usize;

static INBOUND_INVENTORY_BROADCAST_INTERVAL: u64 = 5;
static OUTBOUND_INVENTORY_BROADCAST_INTERVAL: u64 = 2;
static NONPREF_PEER_TX_DELAY: u64 = 2;

macro_rules! debug_log {
    ($time:tt, $id:expr, $($arg:tt)*)
    =>
    (log::debug!("{}: [Node: {}] {}", $time, $id, &format!($($arg)*)));
}

#[derive(Clone)]
pub struct PoissonTimer {
    dist: Exp<f64>,
    next_interval: u64,
}

impl PoissonTimer {
    pub fn new(mean: u64) -> Self {
        Self {
            dist: Exp::new(1.0 / mean as f64).unwrap(),
            next_interval: 0,
        }
    }

    pub fn sample(&mut self, rng: &mut StdRng) -> u64 {
        // Threat samples as nanoseconds
        (self.dist.sample(rng) * SECS_TO_NANOS as f64).round() as u64
    }
}

#[derive(Clone)]
pub struct Peer {
    scheduled_announcements: HashSet<TxId>,
    known_transactions: HashSet<TxId>,
}

impl Peer {
    pub fn new() -> Self {
        Self {
            scheduled_announcements: HashSet::new(),
            known_transactions: HashSet::new(),
        }
    }

    fn knows_transaction(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    fn has_announcement_scheduled(&mut self, txid: &TxId) -> bool {
        self.scheduled_announcements.contains(txid)
    }

    fn schedule_announcement(&mut self, txid: TxId) {
        self.scheduled_announcements.insert(txid);
    }

    fn remove_scheduled_announcement(&mut self, txid: &TxId) {
        self.scheduled_announcements.remove(txid);
    }
}

impl std::default::Default for Peer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct NodeStatistics {
    // sent/received
    inv: (u32, u32),
    get_data: (u32, u32),
    tx: (u32, u32),
}

impl NodeStatistics {
    pub fn new() -> Self {
        NodeStatistics {
            inv: (0, 0),
            get_data: (0, 0),
            tx: (0, 0),
        }
    }

    pub fn add_sent(&mut self, msg: NetworkMessage) {
        match msg {
            NetworkMessage::INV(_) => self.inv.0 += 1,
            NetworkMessage::GETDATA(_) => self.get_data.0 += 1,
            NetworkMessage::TX(_) => self.tx.0 += 1,
        }
    }

    pub fn add_received(&mut self, msg: NetworkMessage) {
        match msg {
            NetworkMessage::INV(_) => self.inv.1 += 1,
            NetworkMessage::GETDATA(_) => self.get_data.1 += 1,
            NetworkMessage::TX(_) => self.tx.1 += 1,
        }
    }
}

impl std::fmt::Display for NodeStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "INVS: {}/{}. GETDATA: {}/{}. TX: {}/{}",
            self.inv.0, self.inv.1, self.get_data.0, self.get_data.1, self.tx.0, self.tx.1,
        )
    }
}

#[derive(Clone)]
pub struct Node {
    node_id: NodeId,
    rng: StdRng,
    is_reachable: bool,
    in_peers: HashMap<NodeId, Peer>,
    out_peers: HashMap<NodeId, Peer>,
    requested_transactions: HashSet<TxId>,
    delayed_requests: HashMap<TxId, NodeId>,
    known_transactions: HashSet<TxId>,
    inbounds_poisson_timer: PoissonTimer,
    outbounds_poisson_timers: HashMap<NodeId, PoissonTimer>,
    node_statistics: NodeStatistics,
}

impl Node {
    pub fn new(node_id: NodeId, rng: StdRng, is_reachable: bool) -> Self {
        Node {
            node_id,
            rng,
            is_reachable,
            in_peers: HashMap::new(),
            out_peers: HashMap::new(),
            requested_transactions: HashSet::new(),
            delayed_requests: HashMap::new(),
            known_transactions: HashSet::new(),
            inbounds_poisson_timer: PoissonTimer::new(INBOUND_INVENTORY_BROADCAST_INTERVAL),
            outbounds_poisson_timers: HashMap::new(),
            node_statistics: NodeStatistics::new(),
        }
    }

    pub fn get_next_announcement_time(
        &mut self,
        current_time: u64,
        peer_id: Option<NodeId>,
    ) -> u64 {
        let is_inbound = peer_id.is_none();
        let poisson_timer = if is_inbound {
            &mut self.inbounds_poisson_timer
        } else {
            let peer_id = peer_id.unwrap_or_else(|| {
                panic!("Trying to schedule a new sample interval for a not specified outbound peer")
            });
            self.outbounds_poisson_timers
                .get_mut(&peer_id).unwrap_or_else(|| panic!("Trying to schedule a new sample interval for a peer we are not connected to {}", peer_id))
        };

        if current_time >= poisson_timer.next_interval {
            if is_inbound {
                debug_log!(
                    current_time,
                    self.node_id,
                    "Updating inbounds poisson timer"
                );
            } else {
                debug_log!(
                    current_time,
                    self.node_id,
                    "Updating poisson timer for peer {}",
                    peer_id.unwrap()
                );
            }
            poisson_timer.next_interval = current_time + poisson_timer.sample(&mut self.rng);
        }
        poisson_timer.next_interval
    }

    fn get_peer_mut(&mut self, peer_id: &NodeId) -> Option<&mut Peer> {
        self.out_peers
            .get_mut(peer_id)
            .or_else(|| self.in_peers.get_mut(peer_id))
    }

    pub fn get_id(&self) -> NodeId {
        self.node_id
    }

    pub fn get_inbounds(&self) -> &HashMap<NodeId, Peer> {
        &self.in_peers
    }

    pub fn get_outbounds(&self) -> &HashMap<NodeId, Peer> {
        &self.out_peers
    }

    pub fn is_inbounds(&self, peer_id: &NodeId) -> bool {
        self.in_peers.contains_key(peer_id)
    }

    pub fn connect(&mut self, peer_id: NodeId) {
        assert!(
            !self.in_peers.contains_key(&peer_id),
            "Peer {peer_id} is already connected to us"
        );
        assert!(
            self.out_peers.insert(peer_id, Peer::new()).is_none(),
            "We ({}) are already connected to {peer_id}",
            self.node_id
        );
        self.outbounds_poisson_timers.insert(
            peer_id,
            PoissonTimer::new(OUTBOUND_INVENTORY_BROADCAST_INTERVAL),
        );
    }

    pub fn accept_connection(&mut self, peer_id: NodeId) {
        assert!(
            self.is_reachable,
            "Node {peer_id} tried to connect to us (node_id: {}), but we are not reachable",
            self.node_id
        );
        assert!(
            !self.out_peers.contains_key(&peer_id),
            "We (node_id: {}) are already connected to peer {peer_id}",
            self.node_id
        );
        assert!(
            self.in_peers
                // Inbounds are all on the same shared timer
                .insert(peer_id, Peer::new())
                .is_none(),
            "Peer {peer_id} is already connected to us"
        );
    }

    pub fn knows_transaction(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    pub fn get_statistics(&self) -> &NodeStatistics {
        &self.node_statistics
    }

    pub fn broadcast_tx(&mut self, txid: TxId, current_time: u64) -> Vec<(Event, u64)> {
        self.add_known_transaction(txid);
        let mut events = Vec::new();

        for peer_id in self.out_peers.keys().cloned().collect::<Vec<_>>() {
            // We are initializing the transaction's originator interval sampling here. This is because
            // we don't want to start sampling until we have something to send to our peers. Otherwise we
            // would create useless events just for sampling. Notice this works since samples from a
            // Poisson process is memoryless (past events do not affect future events)
            let next_interval = self.get_next_announcement_time(current_time, Some(peer_id));
            debug_log!(
                current_time,
                self.node_id,
                "Scheduling inv to peer {peer_id}"
            );
            // Schedule the announcement to go off on the next trickle for the given peer
            events.push((self.schedule_announcement(peer_id, txid), next_interval));
            events.push((
                Event::sample_new_interval(self.node_id, Some(peer_id)),
                next_interval,
            ));
        }

        // For inbounds we use a shared interval
        let next_interval = self.get_next_announcement_time(current_time, None);
        events.push((
            Event::sample_new_interval(self.node_id, None),
            next_interval,
        ));
        for peer_id in self.in_peers.keys().cloned().collect::<Vec<_>>() {
            events.push((self.schedule_announcement(peer_id, txid), next_interval));
        }

        events
    }

    fn schedule_announcement(&mut self, peer_id: NodeId, txid: TxId) -> Event {
        let peer = self.get_peer_mut(&peer_id).unwrap();
        assert!(
            !peer.has_announcement_scheduled(&txid),
            "Node {peer_id} already has an announcement scheduled for transaction (txid: {txid:x})",
        );
        peer.schedule_announcement(txid);
        Event::process_delayed_announcement(self.node_id, peer_id, txid)
    }

    pub fn process_scheduled_announcement(
        &mut self,
        peer_id: NodeId,
        txid: TxId,
        current_time: u64,
    ) -> Option<(Event, u64)> {
        self.get_peer_mut(&peer_id)
            .unwrap()
            .remove_scheduled_announcement(&txid);
        self.send_message_to(NetworkMessage::INV(txid), peer_id, current_time)
    }

    fn add_request(
        &mut self,
        txid: TxId,
        peer_id: NodeId,
        request_time: u64,
    ) -> Option<(Event, u64)> {
        // Transactions are only requested from a single peer (assuming honest behavior)
        // Inbound peers are de-prioritized. If an outbound peer announces a transaction
        // and an inbound peer request is in delayed stage, the inbounds will be dropped and
        // the outbound will be processed
        if !self.knows_transaction(&txid) && !self.requested_transactions.contains(&txid) {
            if self.is_inbounds(&peer_id) {
                if let Entry::Vacant(e) = self.delayed_requests.entry(txid) {
                    e.insert(peer_id);
                    debug_log!(request_time, self.node_id, "Delaying getdata for transaction request (txid: {txid:x}) to peer {peer_id}");
                    return Some((
                        Event::process_delayed_request(self.node_id, txid),
                        request_time + NONPREF_PEER_TX_DELAY * SECS_TO_NANOS,
                    ));
                } else {
                    debug_log!(request_time, self.node_id, "Another delayed getdata for transaction (txid: {txid:x}) has already been scheduled, not requesting to peer_id: {peer_id}");
                }
            } else {
                debug_log!(
                    request_time,
                    self.node_id,
                    "Sending getdata to peer {peer_id}",
                );
                self.requested_transactions.insert(txid);
                self.delayed_requests.remove(&txid);
                return Some((
                    Event::receive_message_from(
                        self.node_id,
                        peer_id,
                        NetworkMessage::GETDATA(txid),
                    ),
                    request_time,
                ));
            }
        } else {
            debug_log!(
                request_time,
                self.node_id,
                "Already known transaction (txid: {txid:x}), not requesting to peer_id: {peer_id}",
            );
        }

        None
    }

    pub fn process_delayed_request(
        &mut self,
        txid: TxId,
        request_time: u64,
    ) -> Option<(Event, u64)> {
        if !self.knows_transaction(&txid) && !self.requested_transactions.contains(&txid) {
            if let Some(peer_id) = self.delayed_requests.remove(&txid) {
                self.requested_transactions.insert(txid);

                let msg = NetworkMessage::GETDATA(txid);
                self.node_statistics.add_sent(msg);
                return Some((
                    Event::receive_message_from(self.node_id, peer_id, msg),
                    request_time,
                ));
            } else {
                panic!("Trying to process a delayed request that was never added to the collection (txid: {}).", txid);
            }
        } else {
            // A delayed request was triggered but it has already been covered by a non-delayed one
            self.delayed_requests.remove(&txid);
        }

        None
    }

    fn remove_request(&mut self, txid: &TxId) {
        assert!(!self.delayed_requests.contains_key(txid));
        assert!(self.requested_transactions.remove(txid));
    }

    pub fn send_message_to(
        &mut self,
        msg: NetworkMessage,
        peer_id: NodeId,
        request_time: u64,
    ) -> Option<(Event, u64)> {
        let we_know_tx = self.knows_transaction(msg.inner());
        let mut message = None;

        if let Some(peer) = self.get_peer_mut(&peer_id) {
            match msg {
                NetworkMessage::INV(txid) => {
                    assert!(we_know_tx, "Trying to announce a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                    // Don't send the transaction to a peer that has already announced it to us
                    if peer.knows_transaction(&txid) {
                        debug_log!(
                            request_time,
                            self.node_id,
                            "Node {peer_id} has already announced transaction {txid:x} to us, not announcing it back",
                        );
                        return None;
                    }
                    debug_log!(
                        request_time,
                        self.node_id,
                        "Sending {msg} to peer {peer_id}"
                    );
                    message = Some((
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ));
                }
                NetworkMessage::GETDATA(txid) => {
                    assert!(!we_know_tx, "Trying to request a transaction we already know about (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                    assert!(peer.knows_transaction(&txid), "Trying to request a transaction (txid: {txid:x}) from a peer that shouldn't know about it (peer_id: {peer_id})");
                    message = self.add_request(txid, peer_id, request_time);
                }
                NetworkMessage::TX(txid) => {
                    assert!(we_know_tx, "Trying to send a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                    assert!(!peer.knows_transaction(&txid), "Trying to send a transaction (txid: {txid:x}) to a peer that already should know about it (peer_id: {peer_id})");
                    peer.add_known_transaction(txid);
                    debug_log!(
                        request_time,
                        self.node_id,
                        "Sending {msg} to peer {peer_id}"
                    );
                    message = Some((
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ));
                }
            }
        }

        // Only update the "sent" node stats if we are creating a "receive from"
        // message event for a peer
        if let Some((event, _)) = &message {
            if event.is_receive_message() {
                self.node_statistics.add_sent(msg);
            }
        }

        message
    }

    pub fn receive_message_from(
        &mut self,
        msg: NetworkMessage,
        peer_id: NodeId,
        request_time: u64,
    ) -> Vec<(Event, u64)> {
        debug_log!(
            request_time,
            self.node_id,
            "Received {msg} from peer {peer_id}"
        );
        let we_know_tx = self.knows_transaction(msg.inner());
        let mut events = Vec::new();
        self.node_statistics.add_received(msg);

        if let Some(peer) = self.get_peer_mut(&peer_id) {
            match msg {
                NetworkMessage::INV(txid) => {
                    peer.add_known_transaction(txid);
                    // We only request transactions that we don't know about
                    if !we_know_tx {
                        debug_log!(
                            request_time,
                            self.node_id,
                            "Transaction unknown. Scheduling getdata to peer {peer_id}",
                        );
                        if let Some(event) = self.send_message_to(
                            NetworkMessage::GETDATA(txid),
                            peer_id,
                            request_time,
                        ) {
                            events.push(event);
                        }
                    } else {
                        debug_log!(
                            request_time,
                            self.node_id,
                            "Already known transaction (txid: {txid:x})"
                        );
                    }
                }
                NetworkMessage::GETDATA(txid) => {
                    assert!(we_know_tx, "Received transaction request for a transaction we don't know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                    assert!(!peer.knows_transaction(&txid), "Received a transaction request (txid: {txid:x}) from a peer that should already know about it (peer_id {peer_id})");
                    if let Some(event) =
                        self.send_message_to(NetworkMessage::TX(txid), peer_id, request_time)
                    {
                        events.push(event);
                    }
                }
                NetworkMessage::TX(txid) => {
                    assert!(!we_know_tx, "Received a transaction we already know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                    assert!(peer.knows_transaction(&txid), "Received a transaction (txid: {txid:x}) from a node that shouldn't know about it (peer_id {peer_id})");
                    self.remove_request(&txid);
                    events.extend(self.broadcast_tx(txid, request_time));
                }
            }
        }

        events
    }
}
