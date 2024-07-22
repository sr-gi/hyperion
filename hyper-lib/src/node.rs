use std::collections::{hash_map::Entry, HashMap, HashSet};

use itertools::Itertools;
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
    to_be_announced: Vec<TxId>,
    /// Transactions already known by this peer, this is populated either when a peer announced a transaction
    /// to us, or when we send a transaction to them
    known_transactions: HashSet<TxId>,
}

impl Peer {
    pub fn new() -> Self {
        Self {
            to_be_announced: Vec::new(),
            known_transactions: HashSet::new(),
        }
    }

    fn knows_transaction(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    pub fn add_tx_to_be_announced(&mut self, txid: TxId) {
        self.to_be_announced.push(txid)
    }

    pub fn drain_txs_to_be_announced(&mut self) -> Vec<TxId> {
        self.to_be_announced.drain(..).collect()
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
    delayed_requests: HashMap<NodeId, Vec<TxId>>,
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

    fn get_peer(&self, peer_id: &NodeId) -> Option<&Peer> {
        self.out_peers
            .get(peer_id)
            .or_else(|| self.in_peers.get(peer_id))
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

    fn filter_known_and_requested_transactions<'a, I>(
        &'a self,
        iter: I,
    ) -> impl Iterator<Item = I::Item> + 'a
    where
        I: Iterator<Item = &'a TxId> + 'a,
    {
        iter.filter(move |txid| {
            !self.knows_transaction(txid) && !self.requested_transactions.contains(txid)
        })
    }

    pub fn get_statistics(&self) -> &NodeStatistics {
        &self.node_statistics
    }

    /// Schedules the announcement of a given transaction to all our peers, based on when the next announcement interval will be.
    /// Inbound peers use a shared Poisson timer with expected value of [INBOUND_INVENTORY_BROADCAST_INTERVAL] seconds,
    /// while outbound have a unique one with expected value of [OUTBOUND_INVENTORY_BROADCAST_INTERVAL] seconds.
    /// Returns a collection of the scheduled events
    pub fn broadcast_tx(&mut self, txid: TxId, current_time: u64) -> Vec<(Event, u64)> {
        self.add_known_transaction(txid);
        let mut events = Vec::new();

        for peer_id in self.out_peers.keys().cloned().collect::<Vec<_>>() {
            // We are initializing the transaction's originator interval sampling here. This is because
            // we don't want to start sampling until we have something to send to our peers. Otherwise we
            // would create useless events just for sampling. Notice this works since a Poisson process is memoryless
            // hence past events do not affect future ones
            let next_interval = self.get_next_announcement_time(current_time, Some(peer_id));
            debug_log!(
                current_time,
                self.node_id,
                "Scheduling inv to peer {peer_id} for time {next_interval}"
            );
            // Schedule the announcement to go off on the next trickle for the given peer
            self.get_peer_mut(&peer_id)
                .unwrap()
                .add_tx_to_be_announced(txid);
            events.push((
                Event::process_scheduled_announcement(self.node_id, peer_id),
                next_interval,
            ));
        }

        // For inbounds we use a shared interval
        let next_interval = self.get_next_announcement_time(current_time, None);
        debug_log!(
            current_time,
            self.node_id,
            "Scheduling inv to inbound peers for time {next_interval}"
        );
        for peer_id in self.in_peers.keys().cloned().collect::<Vec<_>>() {
            self.get_peer_mut(&peer_id)
                .unwrap()
                .add_tx_to_be_announced(txid);
            events.push((
                Event::process_scheduled_announcement(self.node_id, peer_id),
                next_interval,
            ));
        }

        events
    }

    /// Processes a previously scheduled transaction announcement to a given peer, returning an event (and the time at which it should be processed) if successful
    pub fn process_scheduled_announcement(
        &mut self,
        peer_id: NodeId,
        current_time: u64,
    ) -> Option<(Event, u64)> {
        // We assume all transactions pending to be announced fit in a single INV message. This is a simplification.
        // However, an INV message fits up to 50000 entries, so it should be enough for out simulation purposes.
        // https://github.com/bitcoin/bitcoin/blob/20ccb30b7af1c30cece2b0579ba88aa101583f07/src/net_processing.cpp#L87-L88
        let peer = self.get_peer_mut(&peer_id).unwrap();
        // Filter all transactions that the peer already knows (because they have announced them to us already)
        let to_be_announced = peer
            .drain_txs_to_be_announced()
            .into_iter()
            .filter(|txid| !peer.knows_transaction(txid))
            .collect::<Vec<TxId>>();

        if to_be_announced.is_empty() {
            return None;
        } else {
            self.send_message_to(NetworkMessage::INV(to_be_announced), peer_id, current_time)
        }
    }

    /// Adds the request of a set of transactions to a given node to our trackers. If the node is inbounds, this will generate
    /// a delayed request, and return an event to be processed later in time. If the node is outbounds, the request will be generated
    /// straightaway. No request will be generated if we already know all provided transactions
    fn add_request(
        &mut self,
        txids: Vec<TxId>,
        peer_id: NodeId,
        request_time: u64,
    ) -> Option<(Event, u64)> {
        let to_be_requested = self
            .filter_known_and_requested_transactions(txids.iter())
            .map(|x| *x)
            .collect::<Vec<_>>();
        if to_be_requested.is_empty() {
            debug_log!(
                request_time,
                self.node_id,
                "Already known transactions (txids: [{:x}]), not requesting them to peer_id: {peer_id}",
                txids.iter().format(", ")
            );
            return None;
        }

        // Transactions are only requested from a single peer (assuming honest behavior)
        // Inbound peers are de-prioritized. If an outbound peer announces a transaction
        // and an inbound peer request is in delayed stage, the inbounds will be dropped and
        // the outbound will be processed
        if self.is_inbounds(&peer_id) {
            debug_log!(
                request_time,
                self.node_id,
                "Delaying getdata for transactions (txids: [{:x}]) to peer {peer_id}",
                to_be_requested.iter().format(", ")
            );
            for txid in to_be_requested.iter() {
                match self.delayed_requests.entry(peer_id) {
                    Entry::Occupied(mut e) => {
                        e.get_mut().push(*txid);
                    }
                    Entry::Vacant(e) => {
                        e.insert(vec![*txid]);
                    }
                };
            }
            Some((
                Event::process_delayed_request(self.node_id, peer_id),
                request_time + NONPREF_PEER_TX_DELAY * SECS_TO_NANOS,
            ))
        } else {
            debug_log!(
                request_time,
                self.node_id,
                "Sending getdata for transactions (txids: [{:x}]) to peer {peer_id}",
                to_be_requested.iter().format(", ")
            );
            self.requested_transactions.extend(to_be_requested.iter());
            Some((
                Event::receive_message_from(
                    self.node_id,
                    peer_id,
                    NetworkMessage::GETDATA(to_be_requested),
                ),
                request_time,
            ))
        }
    }

    /// Processes a delayed request for a given inbound peer. If any transaction is still pending to be requested, the delayed request
    /// will be processed and an event will be generated. If all transactions have been already requested to outbound peers
    /// during our delay, the request will simply be dropped.
    /// Notice that we do not queue requests (because the nodes is the simulator are well behaved)
    pub fn process_delayed_request(
        &mut self,
        peer_id: NodeId,
        request_time: u64,
    ) -> Option<(Event, u64)> {
        let pending = self.delayed_requests.remove(&peer_id).unwrap_or_else(|| {
            panic!("Trying to process an non-existing delayed request for peer_id: {peer_id}")
        });
        let to_be_requested = self
            .filter_known_and_requested_transactions(pending.iter())
            .map(|x| *x)
            .collect::<Vec<_>>();
        if to_be_requested.is_empty() {
            debug_log!(
                request_time,
                self.node_id,
                "A non-delayed request for transactions (txids: [{:x}]), has already been processed. Not requesting them to peer_id: {peer_id}",
                pending.iter().format(", ")
            );
            return None;
        }

        self.requested_transactions.extend(to_be_requested.iter());
        let msg = NetworkMessage::GETDATA(to_be_requested);
        self.node_statistics.add_sent(&msg, true);

        Some((
            Event::receive_message_from(self.node_id, peer_id, msg),
            request_time,
        ))
    }

    /// Tries so send a message (of a given type) to a given peer, creating the corresponding receive message event if successful.
    /// This may generate a delayed event on ourselves, for instance when tying to send a get data to an inbound peer
    fn send_message_to(
        &mut self,
        msg: NetworkMessage,
        peer_id: NodeId,
        request_time: u64,
    ) -> Option<(Event, u64)> {
        let message: Option<(Event, u64)>;

        if let Some(peer) = self.get_peer(&peer_id) {
            match msg {
                NetworkMessage::INV(ref txids) => {
                    assert!(!txids.is_empty(), "Trying to send out an empty transaction announcement to a peer (peer_id: {peer_id})");
                    for txid in txids.iter() {
                        assert!(self.knows_transaction(txid), "Trying to announce a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                        assert!(!peer.knows_transaction(txid), "Trying to announce a transaction (txid: {txid:x}) to a peer that already knows about it (peer_id: {peer_id})");
                    }

                    message = Some((
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ));
                }
                NetworkMessage::GETDATA(txids) => {
                    assert!(!txids.is_empty(), "Trying to send out an empty transaction request to a peer (peer_id: {peer_id})");
                    for txid in txids.iter() {
                        assert!(!self.knows_transaction(txid), "Trying to request a transaction we already know about (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                        assert!(peer.knows_transaction(txid), "Trying to request a transaction (txid: {txid:x}) from a peer that shouldn't know about it (peer_id: {peer_id})");
                    }

                    message = self.add_request(txids, peer_id, request_time);
                }
                NetworkMessage::TX(txid) => {
                    assert!(self.knows_transaction(&txid), "Trying to send a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                    assert!(!peer.knows_transaction(&txid), "Trying to send a transaction (txid: {txid:x}) to a peer that already should know about it (peer_id: {peer_id})");
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .add_known_transaction(txid);

                    message = Some((
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ));
                }
            }
        } else {
            panic!(
                "Trying to send a message to a node we are not connected to (node_id: {peer_id})"
            )
        }

        // Only update the "sent" node stats if we are creating a "receive from"
        // message event for a peer
        if let Some((event, _)) = &message {
            if let Some(msg) = event.get_message() {
                if event.is_receive_message() {
                    debug_log!(
                        request_time,
                        self.node_id,
                        "Sending {msg} to peer {peer_id}"
                    );
                    self.node_statistics
                        .add_sent(msg, self.is_inbounds(&peer_id));
                }
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
        assert!(
            self.get_peer(&peer_id).is_some(),
            "Received an message from a node we are not connected to (node_id: {peer_id})"
        );
        debug_log!(
            request_time,
            self.node_id,
            "Received {msg} from peer {peer_id}"
        );

        self.node_statistics
            .add_received(&msg, self.is_inbounds(&peer_id));

        // We cannot hold a reference of peer here since we call mutable methods in the same context.
        // Maybe we can work around this by passing a reference to the peer instead of the node id
        // and getting another reference down the line
        match msg {
            NetworkMessage::INV(txids) => {
                assert!(
                    !txids.is_empty(),
                    "Received an empty transaction announcement from peer (peer_id: {peer_id})"
                );
                let mut to_be_requested = Vec::new();
                for txid in txids.into_iter() {
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .add_known_transaction(txid);
                    // We only request transactions that we don't know about
                    if !self.knows_transaction(&txid) {
                        to_be_requested.push(txid)
                    } else {
                        debug_log!(
                            request_time,
                            self.node_id,
                            "Already known transaction (txid: {txid:x})"
                        );
                    }
                }
                if !to_be_requested.is_empty() {
                    self.send_message_to(
                        NetworkMessage::GETDATA(to_be_requested),
                        peer_id,
                        request_time,
                    )
                    .map_or(Vec::new(), |x| vec![x])
                } else {
                    // If all entries have been filtered out there is no point calling [self::send_message_to]
                    Vec::new()
                }
            }
            NetworkMessage::GETDATA(txids) => {
                txids.iter().map(|txid| {
                        assert!(self.knows_transaction(txid), "Received transaction request for a transaction we don't know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                        assert!(!self.get_peer(&peer_id).unwrap().knows_transaction(txid), "Received a transaction request (txid: {txid:x}) from a peer that should already know about it (peer_id {peer_id})");
                        // Send tx cannot return None
                        self.send_message_to(NetworkMessage::TX(*txid), peer_id, request_time).unwrap()
                    }).collect::<Vec<_>>()
            }
            NetworkMessage::TX(txid) => {
                assert!(!self.knows_transaction(&txid), "Received a transaction we already know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                assert!(self.get_peer(&peer_id).unwrap().knows_transaction(&txid), "Received a transaction (txid: {txid:x}) from a node that shouldn't know about it (peer_id {peer_id})");
                self.requested_transactions.remove(&txid);
                self.broadcast_tx(txid, request_time)
            }
        }
    }
}
