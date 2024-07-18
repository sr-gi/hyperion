use std::collections::{hash_map::Entry, HashMap, HashSet};

use rand::rngs::StdRng;
use rand_distr::{Distribution, Exp};

use crate::network::NetworkMessage;
use crate::simulator::Event;
use crate::statistics::NodeStatistics;
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

/// A discrete timer over a poisson distribution. Used to decide when to
/// announce transactions to peers
#[derive(Clone)]
struct PoissonTimer {
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

    /// Sample a new value from the distribution. Return values are
    /// represented as nanoseconds
    pub fn sample(&mut self, rng: &mut StdRng) -> u64 {
        (self.dist.sample(rng) * SECS_TO_NANOS as f64).round() as u64
    }
}

/// A minimal abstraction of a peer
#[derive(Clone)]
pub struct Peer {
    /// Set of transactions pending to be announced. This is populated when a new announcement event
    /// is scheduled, and consumed once the event takes place
    scheduled_announcements: HashSet<TxId>,
    /// Transactions already known by this peer, this is populated either when a peer announced a transaction
    /// to us, or when we send a transaction to them
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

/// A node is the main unit of the simulator. It represents an abstractions of a Bitcoin node and
/// stores all the data required to simulate sending and receiving transactions
#[derive(Clone)]
pub struct Node {
    /// The (global) node identifier
    node_id: NodeId,
    /// A pre-seeded rng to allow reproducing previous simulation results
    rng: StdRng,
    /// Whether the node is reachable or not
    is_reachable: bool,
    /// Map of inbound peers identified by their (global) node identifier
    in_peers: HashMap<NodeId, Peer>,
    /// Map of outbound peers identified by their (global) node identifier
    out_peers: HashMap<NodeId, Peer>,
    /// Set of transactions that have already been requested (but not yet received)
    requested_transactions: HashSet<TxId>,
    /// Set of transactions which request has been delayed (see [Node::add_request])
    delayed_requests: HashMap<TxId, NodeId>,
    /// Set of transaction already known by the node
    known_transactions: HashSet<TxId>,
    /// Poisson timer shared by all inbound peers. Used to decide when to announce transactions to them
    inbounds_poisson_timer: PoissonTimer,
    /// Map of poisson timers for outbound peers. Used to decide when to announce transactions to each of them
    outbounds_poisson_timers: HashMap<NodeId, PoissonTimer>,
    /// Amount of messages of each time the node has sent/received
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

    /// Gets the next discrete time when a transaction announcement needs to be sent to a given peer.
    /// A [peer_id] is required if the query is performed for an outbound peer, otherwise the request is
    /// assumed to be for inbounds. The method will sample a new time if we have reached the old sample,
    /// otherwise, the old sample will be returned.
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

    /// Check whether a given peer is an inbound connection
    fn is_inbounds(&self, peer_id: &NodeId) -> bool {
        self.in_peers.contains_key(peer_id)
    }

    /// Connects to a given peer. This method is used by the simulator to connect nodes between them
    /// alongside its counterpart [Node::accept_connection]
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

    /// Accept a connection from a given peer. This method is used by the simulator to connect nodes between them
    /// alongside its counterpart [Node::connect]
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

    /// Whether we know a given transaction
    pub fn knows_transaction(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    /// Add a given transaction to our known collection
    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    pub fn get_statistics(&self) -> &NodeStatistics {
        &self.node_statistics
    }

    /// Schedules the announcement of a given transaction to all our peers, based on when the next announcement interval will be.
    /// Inbound peers use a shared poisson timer with expected value of [INBOUND_INVENTORY_BROADCAST_INTERVAL] seconds (simulated),
    /// while outbound have a unique one with expected value of [OUTBOUND_INVENTORY_BROADCAST_INTERVAL] seconds.
    /// Returns a collection of the scheduled events, including re-sampling of the time intervals
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

    /// Creates an scheduled announcement for a given transaction to a given peer.
    /// TODO: Notice this creates an announcement **per transaction**, while INVs can actually
    /// hold many transaction on the same message. This will need to be updated when we simulate
    /// broadcasting multiple transactions
    fn schedule_announcement(&mut self, peer_id: NodeId, txid: TxId) -> Event {
        let peer = self.get_peer_mut(&peer_id).unwrap();
        assert!(
            !peer.has_announcement_scheduled(&txid),
            "Node {peer_id} already has an announcement scheduled for transaction (txid: {txid:x})",
        );
        peer.schedule_announcement(txid);
        Event::process_delayed_announcement(self.node_id, peer_id, txid)
    }

    /// Processes a previously scheduled transaction announcement to a given peer, returning an event (and the time at which it should be processed) if successful
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

    /// Adds the request of a given transaction to a given node to our trackers. If the node is inbounds, this will generate
    /// a delayed request, and return an event to be processed later in time. If the node is outbounds, the request will be generated
    /// straightaway. No request will be generated if we already know the transaction
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

    /// Processes a delayed request for a given transaction. If the transaction has not been requested by anyone else, the delayed
    /// request will be moved to a normal request and an event will be generated. If an outbound peer have requested this transaction
    /// during our delay, this will simply be dropped.
    /// Notice that we do not queue requests (because the nodes is the simulator are well behaved)
    pub fn process_delayed_request(
        &mut self,
        txid: TxId,
        request_time: u64,
    ) -> Option<(Event, u64)> {
        if !self.knows_transaction(&txid) && !self.requested_transactions.contains(&txid) {
            if let Some(peer_id) = self.delayed_requests.remove(&txid) {
                self.requested_transactions.insert(txid);

                let msg = NetworkMessage::GETDATA(txid);
                self.node_statistics.add_sent(msg, true);
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

    /// Removes a request for a given transaction from our trackers
    fn remove_request(&mut self, txid: &TxId) {
        assert!(!self.delayed_requests.contains_key(txid));
        assert!(self.requested_transactions.remove(txid));
    }

    /// Tries so send a message (of a given type) to a given peer, creating the corresponding receive message event if successful.
    /// This may generate a delayed event on ourselves, for instance when tying to send a get data to an inbound peer
    fn send_message_to(
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
                self.node_statistics
                    .add_sent(msg, self.is_inbounds(&peer_id));
            }
        }

        message
    }

    /// Receives a given message from a given peer. This is called on us by the simulator when processing events from its event queue.
    /// This may trigger us sending messages to other peers, which will generate a collection of events to be handled by the simulator
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
        self.node_statistics
            .add_received(msg, self.is_inbounds(&peer_id));

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
