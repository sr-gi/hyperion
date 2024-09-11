use std::collections::hash_map::{DefaultHasher, Entry};
use std::collections::{HashMap, HashSet};
use std::hash::Hasher;

use itertools::Itertools;
use rand::rngs::StdRng;
use rand_distr::{Distribution, Exp};

use crate::indexedmap::IndexedMap;
use crate::network::NetworkMessage;
use crate::simulator::Event;
use crate::statistics::NodeStatistics;
use crate::txreconciliation::TxReconciliationState;
use crate::{TxId, SECS_TO_NANOS};

pub type NodeId = usize;

static INBOUND_INVENTORY_BROADCAST_INTERVAL: u64 = 5;
static OUTBOUND_INVENTORY_BROADCAST_INTERVAL: u64 = 2;
static NONPREF_PEER_TX_DELAY: u64 = 2;
static OUTBOUND_FANOUT_DESTINATIONS: u16 = 1;
static INBOUND_FANOUT_DESTINATIONS_FRACTION: f64 = 0.1;
pub static RECON_REQUEST_INTERVAL: u64 = 8;

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
    fn new(mean: u64) -> Self {
        Self {
            dist: Exp::new(1.0 / mean as f64).unwrap(),
            next_interval: 0,
        }
    }

    /// Sample a new value from the distribution. Return values are
    /// represented as nanoseconds
    fn sample(&mut self, rng: &mut StdRng) -> u64 {
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
    /// Transaction reconciliation related data for peers that support Erlay
    tx_reconciliation_state: Option<TxReconciliationState>,
}

impl Peer {
    fn new(is_erlay: bool, is_inbound: bool) -> Self {
        let tx_reconciliation_state = if is_erlay {
            // Connection initiators match reconciliation initiators
            // https://github.com/bitcoin/bips/blob/master/bip-0330.mediawiki#sendtxrcncl
            Some(TxReconciliationState::new(is_inbound))
        } else {
            None
        };
        Self {
            to_be_announced: Vec::new(),
            known_transactions: HashSet::new(),
            tx_reconciliation_state,
        }
    }

    pub fn knows_transaction(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
        if let Some(recon_state) = self.get_tx_reconciliation_state_mut() {
            recon_state.remove_tx(&txid);
        }
    }

    fn add_tx_to_be_announced(&mut self, txid: TxId) {
        self.to_be_announced.push(txid)
    }

    fn drain_txs_to_be_announced(&mut self) -> Vec<TxId> {
        self.to_be_announced.drain(..).collect()
    }

    fn is_erlay(&self) -> bool {
        self.tx_reconciliation_state.is_some()
    }

    fn add_tx_to_reconcile(&mut self, txid: TxId) -> bool {
        self.tx_reconciliation_state
            .as_mut()
            .map(|recon_set| recon_set.add_tx(txid))
            .unwrap_or(false)
    }

    fn get_tx_reconciliation_state(&self) -> Option<&TxReconciliationState> {
        self.tx_reconciliation_state.as_ref()
    }

    fn get_tx_reconciliation_state_mut(&mut self) -> Option<&mut TxReconciliationState> {
        self.tx_reconciliation_state.as_mut()
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
    /// Whether the node supports Erlay or not
    is_erlay: bool,
    /// Map of inbound peers identified by their (global) node identifier
    in_peers: HashMap<NodeId, Peer>,
    /// Map of outbound peers identified by their (global) node identifier
    out_peers: IndexedMap<NodeId, Peer>,
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
    pub fn new(node_id: NodeId, rng: StdRng, is_reachable: bool, is_erlay: bool) -> Self {
        Node {
            node_id,
            rng,
            is_reachable,
            is_erlay,
            in_peers: HashMap::new(),
            out_peers: IndexedMap::new(),
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
    pub fn get_next_announcement_time(&mut self, current_time: u64, peer_id: NodeId) -> u64 {
        let is_inbound = self.is_peer_inbounds(&peer_id);
        let poisson_timer = if is_inbound {
            &mut self.inbounds_poisson_timer
        } else {
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
                    peer_id
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
        self.out_peers.inner()
    }

    /// Check whether a given peer is an inbound connection
    fn is_peer_inbounds(&self, peer_id: &NodeId) -> bool {
        self.in_peers.contains_key(peer_id)
    }

    /// Connects to a given peer. This method is used by the simulator to connect nodes between them
    /// alongside its counterpart [Node::accept_connection]
    pub fn connect(&mut self, peer_id: NodeId, is_erlay: bool) {
        if is_erlay {
            assert!(self.is_erlay, "We are trying to stablish an Erlay connection with node {peer_id}, but we don't support Erlay");
        }
        assert!(
            !self.in_peers.contains_key(&peer_id),
            "Peer {peer_id} is already connected to us"
        );
        assert!(
            self.out_peers
                .insert(peer_id, Peer::new(is_erlay, false))
                .is_none(),
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
    pub fn accept_connection(&mut self, peer_id: NodeId, is_erlay: bool) {
        if is_erlay {
            assert!(self.is_erlay, "Node {peer_id} is trying to stablish an Erlay connection with us, but we don't support it");
        }
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
                .insert(peer_id, Peer::new(is_erlay, true))
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

    fn is_fanout_target(&self, txid: &TxId, peer_id: &NodeId, n: f64) -> bool {
        // Seed our hasher using the target transaction id
        let mut deterministic_randomizer = DefaultHasher::new();
        deterministic_randomizer.write_u32(*txid);

        // Also add out own node_id as seed. This is only necessary in the simulator given node_is
        // are global. If we don't seed the hasher with a distinct value, two nodes sharing the same
        // neighbourhood will obtain the exact same ordering of peers here
        deterministic_randomizer.write_usize(self.node_id);

        // Round n deterministically at random (this is only really necessary for inbounds, as outbounds always
        // provide an already rounded n)
        let target_size = ((deterministic_randomizer.finish() & 0xFFFFFFFF)
            + (n * 0x100000000u64 as f64) as u64)
            >> 32;

        // Shortcut to prevent useless work
        if target_size == 0 {
            return false;
        }

        // Inbound peers are also initiators
        let peers = if self.is_peer_inbounds(peer_id) {
            self.get_inbounds()
        } else {
            self.get_outbounds()
        };

        // Sort peers deterministically at random so we can pick the ones to fanout to.
        // The order is computed using the hasher, as in whatever value results from
        // hashing (txid | node_id | peer_id)
        let mut best_peers = peers
            .keys()
            .map(|peer_id| {
                // Sort values based on the already seeded hasher and their node_id
                // Clone it so each hasher is only seeded with txid | node_id
                let mut h = deterministic_randomizer.clone();
                h.write_usize(*peer_id);
                (h.finish(), *peer_id)
            })
            .collect::<Vec<_>>();
        best_peers.sort();

        let fanout_candidates = best_peers.iter().map(|(_, y)| *y).collect::<Vec<_>>();
        fanout_candidates[0..target_size as usize].contains(peer_id)
    }

    /// Whether we should fanout the given transaction to the given peer.
    /// Assume that Erlay support is a boolean flag for all nodes in the simulation
    /// for now, so there are no fanout_tx_relay peers if Erlay is supported
    fn should_fanout_to(&self, txid: &TxId, peer_id: &NodeId) -> bool {
        let peer = self.get_peer(peer_id).unwrap();
        if !peer.is_erlay() {
            return true;
        }

        let n = if peer.get_tx_reconciliation_state().unwrap().is_initiator() {
            self.get_inbounds().len() as f64 * INBOUND_FANOUT_DESTINATIONS_FRACTION
        } else {
            OUTBOUND_FANOUT_DESTINATIONS as f64
        };

        self.is_fanout_target(txid, peer_id, n)
    }

    /// Schedules a transaction announcement to be processed at a future time.
    /// We are initializing the transaction's originator interval sampling here. This is because we don't want to start
    /// sampling until we have something to send to our peers. Otherwise we would create useless events just for sampling.
    /// Notice this works since a Poisson process is memoryless hence past events do not affect future ones.
    fn schedule_tx_announcement(
        &mut self,
        txid: TxId,
        peers: Vec<NodeId>,
        current_time: u64,
    ) -> Vec<(Event, u64)> {
        let mut events = Vec::new();
        for peer_id in peers {
            // Do not send the transaction to peers that already know about it (e.g. the peer that sent it to us)
            if !self.get_peer(&peer_id).unwrap().knows_transaction(&txid) {
                if self.should_fanout_to(&txid, &peer_id) {
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .add_tx_to_be_announced(txid);
                } else {
                    // If this peer have been picked for set reconciliation, we can assume the tx_reconciliation_state is set
                    debug_log!(
                        current_time,
                        self.node_id,
                        "Added tx (txids: [{:x}]) to reconciliation set for peer (peer_id: {peer_id})", txid
                    );
                    assert!(
                        self.get_peer_mut(&peer_id)
                            .unwrap()
                            .add_tx_to_reconcile(txid),
                        "Couldn't add tx (txids: [{:x}]) to reconset (peer_id: {peer_id})",
                        txid
                    );
                }
            }

            let next_interval = self.get_next_announcement_time(current_time, peer_id);
            debug_log!(
                current_time,
                self.node_id,
                "Scheduling inv to peer {peer_id} for time {next_interval}"
            );
            // Schedule the announcement to go off on the next trickle for the given peer
            // Notice reconciliation requests are not on a poisson timer, they are triggered every fix interval.
            // However, transactions are made available to reconcile following the peer's poisson timer (check
            // [TxReconciliationState::make_snapshot])
            events.push((
                Event::process_scheduled_announcement(self.node_id, peer_id),
                next_interval,
            ));
        }
        events
    }

    /// Kickstarts the broadcasting logic for a given transaction to all the node's peers. This includes both fanout and transaction
    /// reconciliation. For peers selected for fanout, an announcement is scheduled based on the peers type.
    /// Inbound peers use a shared Poisson timer with expected value of [INBOUND_INVENTORY_BROADCAST_INTERVAL] seconds,
    /// while outbound have a unique one with expected value of [OUTBOUND_INVENTORY_BROADCAST_INTERVAL] seconds.
    /// For peers selected for set reconciliation, this transaction is added to their reconciliation sets and will be made available
    /// when the next announcement is processed (this does not generate an event)
    /// Returns a collection of the scheduled events
    pub fn broadcast_tx(&mut self, txid: TxId, current_time: u64) -> Vec<(Event, u64)> {
        self.add_known_transaction(txid);

        let mut events = self.schedule_tx_announcement(
            txid,
            self.out_peers.keys().cloned().collect::<Vec<_>>(),
            current_time,
        );
        events.extend(self.schedule_tx_announcement(
            txid,
            self.in_peers.keys().cloned().collect::<Vec<_>>(),
            current_time,
        ));

        events
    }

    /// Reconciliation requests, as opposed to INVs, are sent on a schedule, no matter if we have transactions to reconcile or not
    /// (since our peer may have and we are unaware of this). Schedule the next reconciliation request.
    pub fn schedule_set_reconciliation(&mut self, request_time: u64) -> (Event, u64) {
        // Make it so we reconcile with all peers every RECON_REQUEST_INTERVAL
        let delta = ((RECON_REQUEST_INTERVAL as f64 / self.out_peers.len() as f64)
            * SECS_TO_NANOS as f64)
            .round() as u64;
        (
            Event::process_scheduled_reconciliation(self.node_id),
            request_time + delta,
        )
    }

    /// Processes a previously scheduled reconciliation request. This starts the set reconciliation dance
    pub fn process_scheduled_reconciliation(
        &mut self,
        request_time: u64,
    ) -> ((Event, u64), (Event, u64)) {
        // Peers are selected for set reconciliation in a round robin manner. This is managed internally by our IndexedMap
        let (next_peer_id, next_peer) = self.out_peers.get_next();
        debug_log!(
            request_time,
            self.node_id,
            "Requesting transaction reconciliation to peer_id: {next_peer_id}",
        );

        let to_be_reconciled = next_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set()
            .iter()
            .copied()
            .collect_vec();

        // Return two events, one starting the reconciliation flow for the given peer, and another one
        // scheduling the next reconciliation request (for the next peer in line)
        (
            self.send_message_to(
                NetworkMessage::REQRECON(to_be_reconciled),
                next_peer_id,
                request_time,
            )
            .unwrap(),
            self.schedule_set_reconciliation(request_time),
        )
    }

    /// Processes a previously scheduled transaction announcement to a given peer, returning an event (and the time at which it should be processed) if successful
    pub fn process_scheduled_announcement(
        &mut self,
        peer_id: NodeId,
        current_time: u64,
    ) -> Option<(Event, u64)> {
        let peer = self.get_peer_mut(&peer_id).unwrap();

        // Make transactions that could have been announced via fanout available for reconciliation. Transactions added to the
        // reconciliation set between trickles are not available until the next interval
        if peer.is_erlay() {
            peer.get_tx_reconciliation_state_mut()
                .unwrap()
                .make_delayed_available();
        }

        // We assume all transactions pending to be announced fit in a single INV message. This is a simplification.
        // However, an INV message fits up to 50000 entries, so it should be enough for out simulation purposes.
        // https://github.com/bitcoin/bitcoin/blob/20ccb30b7af1c30cece2b0579ba88aa101583f07/src/net_processing.cpp#L87-L88
        let to_be_announced = peer
            .drain_txs_to_be_announced()
            .into_iter()
            // Filter all transactions that the peer already knows (because they have announced them to us already)
            .filter(|txid| !peer.knows_transaction(txid))
            .collect::<Vec<TxId>>();

        if to_be_announced.is_empty() {
            None
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
            .copied()
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
        if self.is_peer_inbounds(&peer_id) {
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
            .copied()
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
        if msg.is_erlay() {
            assert!(
                self.is_erlay,
                "Trying to send an Erlay message to a peer (node_id: {peer_id}), but we do not support Erlay"
            );
            assert!(
                self.get_peer(&peer_id).unwrap().is_erlay(),
                "Trying to send an Erlay message to a peer (node_id: {peer_id}), but the it hasn't signaled Erlay support"
            );
        }
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
                NetworkMessage::REQRECON(ref txids) => {
                    assert!(self.is_erlay, "Trying to send a reconciliation request to peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(self.get_peer(&peer_id).unwrap().is_erlay(), "Trying to send a reconciliation request to peer (peer_id: {peer_id}) but they do not support Erlay");
                    for txid in txids.iter() {
                        assert!(self.knows_transaction(txid), "Trying to reconcile a transaction we should't know about (txid: {txid:x}) with a peer (peer_id: {peer_id})");
                        // If we know that the peer knows, we have received at least an announcement from the peer (if not the transaction itself) meaning that the transaction shouldn't be
                        // in this peer's reconciliation set
                        assert!(!peer.knows_transaction(txid), "Trying to reconcile a transaction (txid: {txid:x}) with a peer that already knows about it (peer_id: {peer_id})");
                    }
                    assert!(!peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Trying to send a reconciliation request to a peer we are already reconciling with (peer_id: {peer_id})");
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .get_tx_reconciliation_state_mut()
                        .unwrap()
                        .set_reconciling();
                    message = Some((
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ))
                }
                NetworkMessage::SKETCH(ref sketch) => {
                    assert!(self.is_erlay, "Trying to send a reconciliation sketch to peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(self.get_peer(&peer_id).unwrap().is_erlay(), "Trying to send a reconciliation sketch to peer (peer_id: {peer_id}) but they do not support Erlay");
                    assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Trying to send a reconciliation sketch to a peer that hasn't requested so (peer_id: {peer_id})");
                    // This check is a bit of a hack, but in the simulator short_ids and txids match
                    for txid in sketch.get_tx_set() {
                        assert!(self.knows_transaction(txid), "Trying to send a sketch containing transaction (txid: {txid:x}) to a peer (peer_id: {peer_id}) but we should't know about it");
                    }
                    message = Some((
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ))
                }
                NetworkMessage::RECONCILDIFF(ref diff) => {
                    assert!(self.is_erlay, "Trying to send a reconciliation difference to peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(self.get_peer(&peer_id).unwrap().is_erlay(), "Trying to send a reconciliation difference to peer (peer_id: {peer_id}) but they do not support Erlay");
                    assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Trying to send a reconciliation difference to a peer that hasn't requested so (peer_id: {peer_id})");
                    for txid in diff.iter() {
                        assert!(!self.knows_transaction(txid), "Trying to request a transaction we should know about (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                    }
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .get_tx_reconciliation_state_mut()
                        .unwrap()
                        .clear();
                    message = Some((
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ))
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
                        .add_sent(msg, self.is_peer_inbounds(&peer_id));
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
        if msg.is_erlay() {
            assert!(
                self.is_erlay,
                "Received an Erlay message from a peer (node_id: {peer_id}), but we do not support Erlay"
            );
            assert!(
                self.get_peer(&peer_id).unwrap().is_erlay(),
                "Received an Erlay message from a peer (node_id: {peer_id}), but the it hasn't signaled Erlay support"
            );
        }
        debug_log!(
            request_time,
            self.node_id,
            "Received {msg} from peer {peer_id}"
        );

        self.node_statistics
            .add_received(&msg, self.is_peer_inbounds(&peer_id));

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
            NetworkMessage::REQRECON(txids) => {
                assert!(self.is_erlay, "Received a reconciliation request from peer (peer_id: {peer_id}) but we do not support Erlay");
                assert!(self.get_peer(&peer_id).unwrap().is_erlay(), "Received a reconciliation request from peer (peer_id: {peer_id}) but they do not support Erlay");
                let peer = self.get_peer_mut(&peer_id).unwrap();
                assert!(!peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Received a reconciliation request from a peer we are already reconciling with (peer_id: {peer_id})");
                peer.get_tx_reconciliation_state_mut()
                    .unwrap()
                    .set_reconciling();
                let sketch = peer
                    .get_tx_reconciliation_state_mut()
                    .unwrap()
                    .compute_sketch(txids);

                self.send_message_to(NetworkMessage::SKETCH(sketch), peer_id, request_time)
                    .map_or(Vec::new(), |x| vec![x])
            }
            NetworkMessage::SKETCH(their_sketch) => {
                let peer = self.get_peer(&peer_id).unwrap();
                assert!(self.is_erlay, "Received a reconciliation sketch from peer (peer_id: {peer_id}) but we do not support Erlay");
                assert!(peer.is_erlay(), "Received a reconciliation sketch from peer (peer_id: {peer_id}) but they do not support Erlay");
                assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Received a reconciliation sketch from a peer that we haven't requested to reconcile with (peer_id: {peer_id})");

                // Compute the local difference between the sets (what we are missing) and send a RECONCLDIFF message to the peer containing them
                let (local_diff, mut remote_diff) = peer
                    .get_tx_reconciliation_state()
                    .unwrap()
                    .compute_sketch_diff(their_sketch);

                let (local_diff, already_known) = local_diff
                    .into_iter()
                    .partition(|x| !self.knows_transaction(x));
                remote_diff = remote_diff
                    .iter()
                    .filter(|x: &&u32| !peer.knows_transaction(x))
                    .copied()
                    .collect::<Vec<_>>();

                // Flag the peer as knowing all the transaction that we both know. Transaction from any of the other two
                // sets (local_diff and remote_diff) will be flagged either when we send them the transactions or when they
                // send them to us
                for txid in already_known {
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .add_known_transaction(txid);
                }

                // Send a RECONCILDIFF with our difference, and an INV with theirs (if any)
                let mut events = self
                    .send_message_to(
                        NetworkMessage::RECONCILDIFF(local_diff),
                        peer_id,
                        request_time,
                    )
                    .map_or(Vec::new(), |x| vec![x]);
                if !remote_diff.is_empty() {
                    events.push(
                        self.send_message_to(
                            NetworkMessage::INV(remote_diff),
                            peer_id,
                            request_time,
                        )
                        .unwrap(),
                    )
                }

                events
            }
            NetworkMessage::RECONCILDIFF(diff) => {
                let peer = self.get_peer(&peer_id).unwrap();
                assert!(self.is_erlay, "Received a reconciliation difference from peer (peer_id: {peer_id}) but we do not support Erlay");
                assert!(peer.is_erlay(), "Received a reconciliation difference from peer (peer_id: {peer_id}) but they do not support Erlay");
                assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Received a reconciliation difference from a peer that we haven't requested to reconcile with (peer_id: {peer_id})");
                for txid in diff.iter() {
                    assert!(self.knows_transaction(txid), "Received a reconciliation difference from peer (peer_id: {peer_id}) containing a transaction we don't know (txid: {txid:x})");
                    assert!(!peer.knows_transaction(txid), "Received a reconciliation difference from peer (peer_id: {peer_id}) containing a transaction they already know (txid: {txid:x})");
                }

                let peer_mut = self.get_peer_mut(&peer_id).unwrap();
                let recon_set = peer_mut.get_tx_reconciliation_state_mut().unwrap().clear();
                for txid in recon_set.into_iter().filter(|txid| !diff.contains(txid)) {
                    peer_mut.add_known_transaction(txid)
                }

                if !diff.is_empty() {
                    self.send_message_to(NetworkMessage::INV(diff), peer_id, request_time)
                        .map_or(Vec::new(), |x| vec![x])
                } else {
                    Vec::new()
                }
            }
        }
    }
}

#[cfg(test)]
mod test_peer {
    use super::*;
    use crate::test::get_random_txid;

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
    fn test_add_known_tx() {
        let mut fanout_peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        let txid = get_random_txid();
        fanout_peer.add_known_transaction(txid);
        assert!(fanout_peer.knows_transaction(&txid));
        assert!(!fanout_peer.knows_transaction(&get_random_txid()));

        let mut erlay_peer = Peer::new(/*is_erlay=*/ true, /*is_inbound=*/ false);
        let txid = get_random_txid();
        erlay_peer.add_known_transaction(txid);
        assert!(erlay_peer.knows_transaction(&txid));
        assert!(!erlay_peer.knows_transaction(&get_random_txid()));

        // If an erlay peer has some transaction on their pending to be reconciled (either delayed or on the set)
        // and we add them to known, they will be removed from reconciliation
        let recon_tx = get_random_txid();
        let delayed_tx = get_random_txid();
        erlay_peer.add_tx_to_reconcile(recon_tx);
        erlay_peer
            .get_tx_reconciliation_state_mut()
            .unwrap()
            .make_delayed_available();
        erlay_peer.add_tx_to_reconcile(delayed_tx);

        // The transactions are in their corresponding structures before flagging them as known
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set()
            .contains(&recon_tx));
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set()
            .contains(&delayed_tx));

        // And are removed after
        erlay_peer.add_known_transaction(recon_tx);
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set()
            .contains(&recon_tx));
        erlay_peer.add_known_transaction(delayed_tx);
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set()
            .contains(&delayed_tx));
        assert!(erlay_peer.knows_transaction(&recon_tx));
        assert!(erlay_peer.knows_transaction(&delayed_tx));
    }

    #[test]
    fn test_drain_txs_to_be_announced() {
        let mut peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        let mut txs = Vec::new();

        for _ in 0..10 {
            let txid = get_random_txid();
            peer.add_tx_to_be_announced(txid);
            txs.push(txid);
        }
        assert!(peer.to_be_announced.len() == txs.len());
        let drained_txs = peer.drain_txs_to_be_announced();
        assert_eq!(txs, drained_txs);
        assert!(peer.to_be_announced.is_empty());
    }

    #[test]
    fn test_add_tx_to_reconcile() {
        // Fanout peers have no reconciliation state, so data cannot be added to it
        let mut fanout_peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        assert!(!fanout_peer.add_tx_to_reconcile(get_random_txid()));

        // Erlay peers do have reconciliation state, independently of whether they are initiators or not. Data added
        // to the set is put on the delayed collection first, and moved to the actual set on demand
        let mut erlay_peer = Peer::new(/*is_erlay=*/ true, /*is_inbound=*/ false);
        let txid = get_random_txid();
        assert!(erlay_peer.add_tx_to_reconcile(txid));
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set()
            .contains(&txid));
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set()
            .contains(&txid));

        // Make transactions available for reconciliation. This usually happens on the next trickle interval for the peer
        erlay_peer
            .get_tx_reconciliation_state_mut()
            .unwrap()
            .make_delayed_available();

        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set()
            .contains(&txid));
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set()
            .contains(&txid));
    }
}

#[cfg(test)]
mod test_node {
    use crate::test::get_random_txid;

    use super::*;
    use rand::SeedableRng;

    #[test]
    fn test_get_next_announcement_time() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let outbound_peer_ids = 1..10;
        let inbound_peer_ids = outbound_peer_ids.end..20;

        for peer_id in outbound_peer_ids.clone() {
            node.connect(peer_id, true);
        }

        for peer_id in inbound_peer_ids.clone() {
            node.accept_connection(peer_id, true);
        }

        let current_time = 0;

        for ref peer_id in outbound_peer_ids {
            // next_interval is initialized as 0
            assert_eq!(
                node.outbounds_poisson_timers
                    .get(peer_id)
                    .unwrap()
                    .next_interval,
                0
            );
            // Sampling a new interval should return a value geq than current time (mostly greater
            // but the sample could be zero).
            let next_interval = node.get_next_announcement_time(current_time, *peer_id);
            assert!(next_interval >= current_time);
            // Sampling twice with the same current_time should return the exact same value, given we
            // only update if a new sample is needed, and that only happens if next_interval is in the past
            assert!(node.get_next_announcement_time(current_time, *peer_id) == next_interval);

            // Sampling for a new interval (geq next_interval) will give you a new value
            assert!(node.get_next_announcement_time(next_interval, *peer_id) > next_interval);
        }

        assert_eq!(node.inbounds_poisson_timer.next_interval, 0);

        let shared_interval = node.get_next_announcement_time(current_time, inbound_peer_ids.start);
        for ref peer_id in inbound_peer_ids {
            // Same checks as for outbounds
            let next_interval = node.get_next_announcement_time(current_time, *peer_id);
            assert!(next_interval >= current_time);
            assert!(node.get_next_announcement_time(current_time, *peer_id) == next_interval);

            // Also, check that every single returned value is the same, given inbounds share a timer
            assert!(next_interval >= shared_interval);

            // Sampling for a new interval (geq next_interval) will give you a new value
            assert!(node.get_next_announcement_time(next_interval, *peer_id) > next_interval);
        }
    }

    #[test]
    fn test_connections() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let outbound_peer_ids = 1..10;
        let inbound_peer_ids = outbound_peer_ids.end..20;

        for peer_id in outbound_peer_ids.clone() {
            node.connect(peer_id, true);
            assert!(node.out_peers.contains_key(&peer_id));
            assert!(node.outbounds_poisson_timers.contains_key(&peer_id));
        }

        for peer_id in inbound_peer_ids.clone() {
            node.accept_connection(peer_id, true);
            assert!(node.in_peers.contains_key(&peer_id));
        }
    }

    #[test]
    fn test_filter_known_and_requested_transactions() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let mut known_txs = Vec::new();
        let mut unknown_txs = Vec::new();
        let mut requested_txs = Vec::new();

        for _ in 0..10 {
            let known = get_random_txid();
            let requested = get_random_txid();
            node.add_known_transaction(known);
            known_txs.push(known);
            node.requested_transactions.insert(requested);
            requested_txs.push(requested);
            unknown_txs.push(get_random_txid())
        }

        assert_eq!(
            node.filter_known_and_requested_transactions(known_txs.iter())
                .count(),
            0
        );
        assert_eq!(
            node.filter_known_and_requested_transactions(requested_txs.iter())
                .count(),
            0
        );
        assert_eq!(
            node.filter_known_and_requested_transactions(unknown_txs.iter())
                .copied()
                .collect::<Vec<_>>(),
            unknown_txs
        );
        assert_eq!(
            node.filter_known_and_requested_transactions(
                [known_txs.clone(), requested_txs].concat().iter()
            )
            .count(),
            0
        );
        assert_eq!(
            node.filter_known_and_requested_transactions(
                [known_txs, unknown_txs.clone()].concat().iter()
            )
            .copied()
            .collect::<Vec<_>>(),
            unknown_txs
        );
    }

    #[test]
    fn test_schedule_tx_announcement_no_erlay() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);

        let outbound_peer_ids = Vec::from_iter(1..11);
        for peer_id in outbound_peer_ids.iter() {
            node.connect(*peer_id, false);
        }

        // For non-erlay peers, schedule_tx_announcement creates an event for each peer
        // This is completely independent of whether the peer is inbound or outbound
        let txid = get_random_txid();
        let events = node.schedule_tx_announcement(txid, outbound_peer_ids.clone(), 0);
        assert_eq!(events.len(), outbound_peer_ids.len());
        for (e, _) in events {
            assert!(matches!(e, Event::ProcessScheduledAnnouncement(..)));
            if let Event::ProcessScheduledAnnouncement(src, dst) = e {
                assert_eq!(src, node_id);
                assert!(outbound_peer_ids.contains(&dst));
                assert!(node
                    .out_peers
                    .get(&dst)
                    .unwrap()
                    .to_be_announced
                    .contains(&txid))
            };
        }
    }

    #[test]
    fn test_schedule_tx_announcement_erlay() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let mut fanout_count = 0;
        let mut reconciliation_count = 0;
        let current_time = 0;

        let outbound_peer_ids = Vec::from_iter(1..11);
        for peer_id in outbound_peer_ids.iter() {
            node.connect(*peer_id, true);
        }

        // For Erlay peers, transactions are added to to_be_announced or to the peer reconciliation set
        // depending on whether or not the peer is selected for fanout. The decision making is performed by
        // should_fanout_to. Here, inbound and outbound peers only change the likelihood of being selected
        let txid = get_random_txid();
        let events = node.schedule_tx_announcement(txid, outbound_peer_ids.clone(), current_time);
        assert_eq!(events.len(), outbound_peer_ids.len());
        for (e, t) in events {
            assert!(matches!(e, Event::ProcessScheduledAnnouncement(..)));
            // Events are all in the future
            assert!(t > current_time);
            if let Event::ProcessScheduledAnnouncement(src, dst) = e {
                assert_eq!(src, node_id);
                assert!(outbound_peer_ids.contains(&dst));
                // The transaction is added to to_be_announced or to the recon_set depending on should_fanout_to
                // This is deterministic, so we can call it again and check
                if node.should_fanout_to(&txid, &dst) {
                    assert!(node
                        .out_peers
                        .get(&dst)
                        .unwrap()
                        .to_be_announced
                        .contains(&txid));
                    fanout_count += 1;
                } else {
                    assert!(node
                        .out_peers
                        .get(&dst)
                        .unwrap()
                        .get_tx_reconciliation_state()
                        .unwrap()
                        .get_delayed_set()
                        .contains(&txid));
                    reconciliation_count += 1;
                }
            };
        }

        // With 10 outbound peers, at least 1 should have been chosen for fanout
        assert!((1..10).contains(&fanout_count));
        assert!((1..10).contains(&reconciliation_count));
    }

    #[test]
    fn test_broadcast_tx() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let outbound_peer_ids = Vec::from_iter(1..11);
        let inbound_peer_ids = Vec::from_iter(11..21);
        let current_time = 0;

        for peer_id in outbound_peer_ids.iter() {
            node.connect(*peer_id, true);
        }

        for peer_id in inbound_peer_ids.iter() {
            node.accept_connection(*peer_id, true);
        }

        let txid = get_random_txid();
        let events = node.broadcast_tx(txid, 0);
        assert!(node.knows_transaction(&txid));

        for (e, t) in events {
            // Events are all in the future
            assert!(t > current_time);
            assert!(matches!(e, Event::ProcessScheduledAnnouncement(..)));
            if let Event::ProcessScheduledAnnouncement(src, dst) = e {
                assert_eq!(src, node_id);
                assert!(outbound_peer_ids.contains(&dst) || inbound_peer_ids.contains(&dst));
            }
        }
    }

    #[test]
    fn test_schedule_set_reconciliation() {
        let node_id = 0;
        let current_time = 0;

        // schedule_set_reconciliation works so we reconcile with all our outbound peers once every RECON_REQUEST_INTERVAL.
        // So each peer is scheduled to be reconciled with every RECON_REQUEST_INTERVAL/outbound_peers.len()

        // Try different number of outbound peers and check how the schedule matches the expectation
        let outbound_peers_sizes = [2, 4, 8];
        for peers_size in outbound_peers_sizes.iter() {
            let mut node = Node::new(node_id, StdRng::seed_from_u64(0), true, true);
            // Connect the desired amount of nodes
            let outbound_peer_ids = Vec::from_iter(0..*peers_size);
            for peer_id in outbound_peer_ids.into_iter() {
                node.connect(peer_id, true);
            }

            // Check the schedule (we use request_time=0 for simplicity)
            let (e, t) = node.schedule_set_reconciliation(current_time);
            assert!(matches!(e, Event::ProcessScheduledReconciliation(..)));
            assert_eq!(
                t,
                (RECON_REQUEST_INTERVAL / *peers_size as u64) * SECS_TO_NANOS
            )
        }
    }

    #[test]
    fn test_process_scheduled_reconciliation() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let outbound_peer_ids = Vec::from_iter(1..11);
        let mut to_be_reconciled = HashMap::new();
        let current_time = 0;

        for peer_id in outbound_peer_ids.iter() {
            node.connect(*peer_id, true);

            // Add a transaction to the reconciliation set of the peer (skip delayed)
            let short_id = get_random_txid();
            to_be_reconciled.insert(peer_id, vec![short_id]);
            node.add_known_transaction(short_id);

            let recon_set_mut = node
                .get_peer_mut(peer_id)
                .unwrap()
                .get_tx_reconciliation_state_mut()
                .unwrap();
            recon_set_mut.add_tx(short_id);
            recon_set_mut.make_delayed_available();
        }

        // Iterating over again to process the reconciliations because we want peers
        // to be connected already
        for peer_id in outbound_peer_ids.iter() {
            let (req_recon, scheduled_recon) = node.process_scheduled_reconciliation(current_time);

            // Check that we receive the two events we are expecting, and that reconciliation request (former event)
            // contains the corresponding short id for each peer
            assert!(matches!(req_recon.0, Event::ReceiveMessageFrom(..)));
            assert!(matches!(
                req_recon.0.get_message().unwrap(),
                NetworkMessage::REQRECON(..)
            ));
            if let NetworkMessage::REQRECON(ids) = req_recon.0.get_message().unwrap() {
                assert_eq!(ids, to_be_reconciled.get(peer_id).unwrap());
            }
            assert!(matches!(
                scheduled_recon.0,
                Event::ProcessScheduledReconciliation(..)
            ))
        }

        // After processing all peers, next_peer should be back to the first
        assert_eq!(
            node.out_peers.get_next().0,
            *outbound_peer_ids.first().unwrap()
        );
    }

    #[test]
    fn test_process_scheduled_announcement() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let current_time = 0;

        // We won't need more than one peer to test this
        let peer_id = 1;
        node.connect(peer_id, true);

        // Processing a scheduled announcement with no data to be sent returns nothing
        assert!(node
            .process_scheduled_announcement(peer_id, current_time)
            .is_none());

        // If the peer has data pending to be sent, an INV message containing such data will be returned.
        // Also, if the peer had some data to be reconciled, that data will be made available (moved out of the delayed set)
        let mut to_be_announced = Vec::new();
        let mut to_be_reconciled = HashSet::new();
        for _ in 1..5 {
            let txid_to_announce = get_random_txid();
            to_be_announced.push(txid_to_announce);
            let txid_to_reconcile = get_random_txid();
            to_be_reconciled.insert(txid_to_reconcile);

            // Add some transactions to be announced and reconciled to the peer state
            node.add_known_transaction(txid_to_announce);
            node.add_known_transaction(txid_to_reconcile);
            let peer_mut = node.get_peer_mut(&peer_id).unwrap();
            peer_mut.add_tx_to_be_announced(txid_to_announce);
            peer_mut.add_tx_to_reconcile(txid_to_reconcile);
        }

        // All transactions to be reconciled are delayed
        assert!(node
            .get_peer(&peer_id)
            .unwrap()
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set()
            .is_empty());
        assert_eq!(
            node.get_peer(&peer_id)
                .unwrap()
                .get_tx_reconciliation_state()
                .unwrap()
                .get_delayed_set(),
            &to_be_reconciled
        );

        // The inv event returned matches the transactions pending to be announced
        let inv_event = node
            .process_scheduled_announcement(peer_id, current_time)
            .unwrap();
        assert!(matches!(inv_event.0, Event::ReceiveMessageFrom(..)));
        if let NetworkMessage::INV(ids) = inv_event.0.get_message().unwrap() {
            assert_eq!(ids, &to_be_announced);
        }

        // Transactions to be reconciled have been made available
        assert!(node
            .get_peer(&peer_id)
            .unwrap()
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set()
            .is_empty());
        assert_eq!(
            node.get_peer(&peer_id)
                .unwrap()
                .get_tx_reconciliation_state()
                .unwrap()
                .get_recon_set(),
            &to_be_reconciled
        );
    }

    #[test]
    fn test_add_request() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let current_time = 0;

        // Calling add_request with an empty collection of transactions to be requested returns None
        // No matter what peer this is called for
        assert!(node.add_request(Vec::new(), 0, current_time).is_none());

        // Add one outbound and one inbound peer
        let outbound_id = 1;
        let inbound_id = 2;
        node.connect(outbound_id, true);
        node.accept_connection(inbound_id, true);

        // All transactions we are unaware of will be requested (all of them in this case) to outbound peers
        let txs = (0..5).map(|_| get_random_txid()).collect::<Vec<_>>();
        let (e, t) = node
            .add_request(txs.clone(), outbound_id, current_time)
            .unwrap();
        assert!(e.is_receive_message());
        if let Event::ReceiveMessageFrom(s, d, m) = e {
            assert_eq!(s, node_id);
            assert_eq!(d, outbound_id);
            assert_eq!(m, NetworkMessage::GETDATA(txs.clone()))
        }
        assert_eq!(t, current_time);
        for txid in txs.iter() {
            assert!(node.requested_transactions.contains(txid));
        }

        // If we try to add something we have requested already (the previous set of transactions for instance)
        // We won't generate a new GETDATA event
        assert!(node
            .add_request(txs.clone(), outbound_id, current_time)
            .is_none());
        // This holds even if we try to request it to another peer
        assert!(node
            .add_request(txs.clone(), inbound_id, current_time)
            .is_none());
        // The same applies for transactions we already know
        let known_tx = get_random_txid();
        node.add_known_transaction(known_tx);
        assert!(node
            .add_request(Vec::from([known_tx]), outbound_id, current_time)
            .is_none());

        // If the peer is inbound instead of outbounds, the request is delayed instead of processed straightaway
        let txs = (0..5).map(|_| get_random_txid()).collect::<Vec<_>>();
        assert!(!node.delayed_requests.contains_key(&inbound_id));
        let (e, t) = node
            .add_request(txs.clone(), inbound_id, current_time)
            .unwrap();
        assert_eq!(e, Event::ProcessDelayedRequest(node_id, inbound_id));
        assert!(t > current_time);
        // Transactions are kept in the delayed_requests collection for future processing
        assert!(node.delayed_requests.contains_key(&inbound_id));
        assert_eq!(node.delayed_requests.get(&inbound_id).unwrap(), &txs);
    }

    #[test]
    fn test_process_delayed_request() {
        let rng = StdRng::seed_from_u64(0);
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let current_time = 0;

        let inbound_id = 1;
        node.accept_connection(inbound_id, true);
        // Add a delayed request
        let txs = (0..5).map(|_| get_random_txid()).collect::<Vec<_>>();
        node.add_request(txs.clone(), inbound_id, current_time);
        assert!(node.delayed_requests.contains_key(&inbound_id));
        // Process the delayed request
        let (e, _) = node
            .process_delayed_request(inbound_id, current_time)
            .unwrap();
        assert!(!node.delayed_requests.contains_key(&inbound_id));
        assert!(e.is_receive_message());
        if let Event::ReceiveMessageFrom(s, d, m) = e {
            assert_eq!(s, node_id);
            assert_eq!(d, inbound_id);
            assert_eq!(m, NetworkMessage::GETDATA(txs.clone()))
        }
        for txid in txs.iter() {
            assert!(node.requested_transactions.contains(txid));
        }

        // If some of the transactions are already known (because a non-delayed request containing them was already processed)
        // those won't be part of the GETDATA message
        let txs = (0..5).map(|_| get_random_txid()).collect::<Vec<_>>();
        // Add all of them
        node.add_request(txs.clone(), inbound_id, current_time);
        // Flag one as already known
        node.add_known_transaction(*txs.first().unwrap());
        // Process
        let (e, _) = node
            .process_delayed_request(inbound_id, current_time)
            .unwrap();
        assert!(!node.delayed_requests.contains_key(&inbound_id));
        assert!(e.is_receive_message());
        if let Event::ReceiveMessageFrom(s, d, m) = e {
            assert_eq!(s, node_id);
            assert_eq!(d, inbound_id);
            // The GETDATA contains all transactions but the already known one
            assert_eq!(m, NetworkMessage::GETDATA(Vec::from(&txs[1..])))
        }

        // If all transactions are known when processing the request, the method returns None
        let txs = Vec::from([get_random_txid()]);
        node.add_request(txs.clone(), inbound_id, current_time);
        node.add_request(txs.clone(), inbound_id, current_time);
        // Flag one as already known
        node.add_known_transaction(*txs.first().unwrap());
        assert!(node
            .process_delayed_request(inbound_id, current_time)
            .is_none());
    }

    // [Node::send_message_to] and [Node::receive_message_from] are self tested.
    // They include asserts for every sent/received message  that would make the
    // message flow fail on runtime if anything was wrong
}
