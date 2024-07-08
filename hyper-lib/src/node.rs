use std::collections::{hash_map::Entry, HashMap, HashSet};

use rand::rngs::ThreadRng;
use rand::thread_rng;
use rand_distr::{Distribution, Poisson};

use crate::network::NetworkMessage;
use crate::simulator::Event;
use crate::TxId;

pub type NodeId = u32;

static INBOUND_INVENTORY_BROADCAST_INTERVAL: u64 = 5;
static OUTBOUND_INVENTORY_BROADCAST_INTERVAL: u64 = 2;
static NONPREF_PEER_TX_DELAY: u64 = 2;

#[derive(Clone)]
pub struct PoissonTimer {
    dist: Poisson<f32>,
    rng: ThreadRng,
    next_interval: u64,
}

impl PoissonTimer {
    pub fn new(lambda: u64) -> Self {
        Self {
            dist: Poisson::new(lambda as f32).unwrap(),
            rng: thread_rng(),
            next_interval: 0,
        }
    }

    pub fn sample(&mut self) -> u64 {
        // Threat samples as nanoseconds
        (self.dist.sample(&mut self.rng) * 1_000_000_000.0) as u64
    }
}

#[derive(Clone)]
pub struct Peer {
    known_transactions: HashSet<TxId>,
}

impl Peer {
    pub fn new() -> Self {
        Self {
            known_transactions: HashSet::new(),
        }
    }

    fn knows_transaction(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }
}

impl std::default::Default for Peer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct Node {
    node_id: NodeId,
    is_reachable: bool,
    in_peers: HashMap<NodeId, Peer>,
    out_peers: HashMap<NodeId, Peer>,
    requested_transactions: HashSet<TxId>,
    delayed_requests: HashMap<TxId, NodeId>,
    known_transactions: HashSet<TxId>,
    inbounds_poisson_timer: PoissonTimer,
    outbounds_poisson_timers: HashMap<NodeId, PoissonTimer>,
}

impl Node {
    pub fn new(node_id: NodeId, is_reachable: bool) -> Self {
        Node {
            node_id,
            is_reachable,
            in_peers: HashMap::new(),
            out_peers: HashMap::new(),
            requested_transactions: HashSet::new(),
            delayed_requests: HashMap::new(),
            known_transactions: HashSet::new(),
            inbounds_poisson_timer: PoissonTimer::new(INBOUND_INVENTORY_BROADCAST_INTERVAL),
            outbounds_poisson_timers: HashMap::new(),
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
                log::debug!("(Node {}) updating inbounds poisson timer", self.node_id);
            } else {
                log::debug!(
                    "(Node {}) updating poisson timer for peer {}",
                    self.node_id,
                    peer_id.unwrap()
                );
            }
            poisson_timer.next_interval = current_time + poisson_timer.sample();
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

    pub fn broadcast_tx(&mut self, txid: TxId, current_time: u64) -> Vec<(Event, u64)> {
        self.add_known_transaction(txid);
        let mut events = Vec::new();

        // FIXME: Check if we can avoid clone here
        for peer_id in self.out_peers.clone().into_keys() {
            // We are initializing the interval sampling of the transaction originator here. This is because
            // we don't want to start sampling until we have something to send to our peers. Otherwise we would
            // just create useless events just for sampling.
            let next_interval = self.get_next_announcement_time(current_time, Some(peer_id));
            if let Some((event, t)) =
                self.send_message_to(NetworkMessage::INV(txid), peer_id, next_interval)
            {
                events.push((
                    Event::sample_new_interval(self.node_id, Some(peer_id)),
                    next_interval,
                ));
                events.push((event, t));
            }
        }

        // For inbounds we have a shared interval
        let next_interval = self.get_next_announcement_time(current_time, None);
        events.push((
            Event::sample_new_interval(self.node_id, None),
            next_interval,
        ));
        // FIXME: Check if we can avoid clone here
        for peer_id in self.in_peers.clone().into_keys() {
            if let Some((event, t)) =
                self.send_message_to(NetworkMessage::INV(txid), peer_id, next_interval)
            {
                events.push((event, t));
            }
        }

        events
    }

    fn add_request(
        &mut self,
        txid: TxId,
        peer_id: NodeId,
        current_time: u64,
    ) -> Option<(Event, u64)> {
        // Transactions are only requested from a single peer (assuming honest behavior)
        // Inbound peers are de-prioritized. If an outbound peer announces a transaction
        // and an inbound peer request is in delayed stage, the inbounds will be dropped and
        // the outbound will be processed
        if !self.knows_transaction(&txid) && !self.requested_transactions.contains(&txid) {
            if self.is_inbounds(&peer_id) {
                if let Entry::Vacant(e) = self.delayed_requests.entry(txid) {
                    e.insert(peer_id);
                    log::info!("(Node {}) Delaying getdata for transaction request (txid: {txid:x}) to peer {peer_id}", self.node_id);
                    return Some((
                        Event::process_delayed_request(self.node_id, txid),
                        current_time + NONPREF_PEER_TX_DELAY * 1_000_000_000,
                    ));
                }
            } else {
                log::info!("(Node {}) Sending getdata to peer {peer_id}", self.node_id);
                self.requested_transactions.insert(txid);
                self.delayed_requests.remove(&txid);
                return Some((
                    Event::receive_message_from(
                        self.node_id,
                        peer_id,
                        NetworkMessage::GETDATA(txid),
                    ),
                    current_time,
                ));
            }
        } else {
            log::info!(
                "(Node {}) Already known transaction (txid: {txid:x}), not requesting to (peer_id: {peer_id})",
                self.node_id
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
        let txid = *msg.inner();
        let we_know_tx = self.knows_transaction(&txid);

        if let Some(peer) = self.get_peer_mut(&peer_id) {
            if msg.is_inv() {
                // INVs for the same transaction can get crossed. If this happens break the cycle.
                assert!(we_know_tx, "Trying to announce a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                if peer.knows_transaction(&txid) {
                    log::debug!(
                        "(Node {}) We already know about transaction {txid:x}, not requesting it back to {}",
                        self.node_id,
                        peer_id
                    );
                    return None;
                }
                log::info!("(Node {}) Scheduling {msg} to peer {peer_id}", self.node_id);
                return Some((
                    Event::receive_message_from(self.node_id, peer_id, msg),
                    request_time,
                ));
            } else if msg.is_get_data() {
                assert!(!we_know_tx, "Trying to request a transaction we already know about (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                assert!(peer.knows_transaction(&txid), "Trying to request a transaction (txid: {txid:x}) from a peer that shouldn't know about it (peer_id: {peer_id})");
                return self.add_request(txid, peer_id, request_time);
            } else {
                assert!(we_know_tx, "Trying to send a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                assert!(!peer.knows_transaction(&txid), "Trying to send a transaction (txid: {txid:x}) to a peer that already should know about it (peer_id: {peer_id})");
                peer.add_known_transaction(txid);
                log::info!("(Node {}) Sending {msg} to peer {peer_id}", self.node_id);
                return Some((
                    Event::receive_message_from(self.node_id, peer_id, msg),
                    request_time,
                ));
            }
        }

        None
    }

    pub fn receive_message_from(
        &mut self,
        msg: NetworkMessage,
        peer_id: NodeId,
        request_time: u64,
    ) -> Vec<(Event, u64)> {
        log::info!("(Node {}) Received {msg} from peer {peer_id}", self.node_id);
        let txid = *msg.inner();
        let we_know_tx = self.knows_transaction(&txid);
        let mut events = Vec::new();

        if let Some(peer) = self.get_peer_mut(&peer_id) {
            if msg.is_inv() {
                peer.add_known_transaction(txid);
                // We only request transactions that we don't know about
                if !we_know_tx {
                    log::info!(
                        "(Node {}) Transaction unknown. Scheduling getdata to peer {peer_id}",
                        self.node_id
                    );
                    if let Some(event) =
                        self.send_message_to(NetworkMessage::GETDATA(txid), peer_id, request_time)
                    {
                        events.push(event);
                    }
                } else {
                    log::info!(
                        "(Node {}) Already known transaction (txid: {txid:x})",
                        self.node_id
                    );
                }
            } else if msg.is_get_data() {
                assert!(we_know_tx, "Received transaction request for a transaction we don't know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                assert!(!peer.knows_transaction(&txid), "Received a transaction request (txid: {txid:x}) from a peer that should already know about it (peer_id {peer_id})");
                if let Some(event) =
                    self.send_message_to(NetworkMessage::TX(txid), peer_id, request_time)
                {
                    events.push(event);
                }
            } else {
                assert!(!we_know_tx, "Received a transaction we already know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                assert!(peer.knows_transaction(&txid), "Received a transaction (txid: {txid:x}) from a node that shouldn't know about it (peer_id {peer_id})");
            }
        }

        // The simulator makes sure a transaction is received by a node twice.
        // Therefore, receiving one is a trigger to start broadcasting.
        // Notice we cannot do this while holding a peer reference
        if msg.is_tx() {
            self.remove_request(&txid);
            events.extend(self.broadcast_tx(txid, request_time));
        }

        events
    }
}
