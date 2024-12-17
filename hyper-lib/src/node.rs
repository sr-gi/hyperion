use std::sync::{Arc, Mutex};

use hashbrown::HashMap;
use rand::prelude::IteratorRandom;
use rand::rngs::StdRng;
use rand_distr::{Distribution, Exp};

use crate::indexedmap::IndexedMap;
use crate::network::NetworkMessage;
use crate::simulator::{Event, ScheduledEvent};
use crate::statistics::NodeStatistics;
use crate::txreconciliation::TxReconciliationState;
use crate::SECS_TO_NANOS;

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
    /// Whether the transaction is pending to be announced. This is set when a new
    /// announcement event is scheduled, and consumed once the event takes place
    to_be_announced: bool,
    /// Whether the node already knows the simulated transaction
    knows_transaction: bool,
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
            to_be_announced: false,
            knows_transaction: false,
            tx_reconciliation_state,
        }
    }

    /// Reset the peer state so a new round of the simulation can be run from a clean state
    pub fn reset(&mut self) {
        self.to_be_announced = false;
        self.knows_transaction = false;
        if let Some(recon_state) = self.get_tx_reconciliation_state_mut() {
            recon_state.clear(/*include_delayed=*/ true);
        }
    }

    pub fn knows_transaction(&self) -> bool {
        self.knows_transaction
    }

    fn add_known_transaction(&mut self) {
        self.knows_transaction = true;
        if let Some(recon_state) = self.get_tx_reconciliation_state_mut() {
            recon_state.remove_tx();
        }
    }

    fn add_tx_to_be_announced(&mut self) {
        assert!(!self.knows_transaction());
        self.to_be_announced = true;
    }

    // Clears tx_to_be_announced, returning whether an announcement was pending or not
    fn drain_tx_to_be_announced(&mut self) -> bool {
        let to_be_announced = self.to_be_announced;
        self.to_be_announced = false;

        to_be_announced
    }

    fn is_erlay(&self) -> bool {
        self.tx_reconciliation_state.is_some()
    }

    fn add_tx_to_reconcile(&mut self) -> bool {
        self.tx_reconciliation_state
            .as_mut()
            .map(|recon_set| recon_set.add_tx())
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
/// stores all the data required to simulate sending and receiving the simulated transaction
#[derive(Clone)]
pub struct Node {
    /// The (global) node identifier
    node_id: NodeId,
    /// A pre-seeded rng to allow reproducing previous simulation results
    rng: Arc<Mutex<StdRng>>,
    /// Whether the node is reachable or not
    is_reachable: bool,
    /// Whether the node supports Erlay or not
    is_erlay: bool,
    /// Map of inbound peers identified by their (global) node identifier
    in_peers: HashMap<NodeId, Peer>,
    /// Map of outbound peers identified by their (global) node identifier
    out_peers: IndexedMap<NodeId, Peer>,
    /// Whether the transaction has been requested (but not yet received)
    requested_transaction: bool,
    /// Whether a request for the simulated transaction has been delayed (see [Node::add_request])
    delayed_request: Option<NodeId>,
    /// Whether the transaction is already known by the node
    known_transaction: bool,
    /// Poisson timer shared by all inbound peers. Used to decide when to announce transactions to them
    inbounds_poisson_timer: PoissonTimer,
    /// Map of poisson timers for outbound peers. Used to decide when to announce transactions to each of them
    outbounds_poisson_timers: HashMap<NodeId, PoissonTimer>,
    /// Amount of messages of each time the node has sent/received
    node_statistics: NodeStatistics,
}

impl Node {
    pub fn new(
        node_id: NodeId,
        rng: Arc<Mutex<StdRng>>,
        is_reachable: bool,
        is_erlay: bool,
    ) -> Self {
        Node {
            node_id,
            rng,
            is_reachable,
            is_erlay,
            in_peers: HashMap::new(),
            out_peers: IndexedMap::new(),
            requested_transaction: false,
            delayed_request: None,
            known_transaction: false,
            inbounds_poisson_timer: PoissonTimer::new(INBOUND_INVENTORY_BROADCAST_INTERVAL),
            outbounds_poisson_timers: HashMap::new(),
            node_statistics: NodeStatistics::new(),
        }
    }

    /// Whether the node is reachable or not
    pub fn is_reachable(&self) -> bool {
        self.is_reachable
    }

    /// Whether the node is erlay or not
    pub fn is_erlay(&self) -> bool {
        self.is_erlay
    }

    // Resets the node state so a new round of the simulation can be run from a clean state
    pub fn reset(&mut self) {
        self.inbounds_poisson_timer.next_interval = 0;
        for timer in self.outbounds_poisson_timers.values_mut() {
            timer.next_interval = 0;
        }

        self.requested_transaction = false;
        self.delayed_request = None;
        self.known_transaction = false;

        for in_peer in self.get_inbounds_mut().values_mut() {
            in_peer.reset();
        }

        for out_peer in self.get_outbounds_mut().values_mut() {
            out_peer.reset();
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
            poisson_timer.next_interval =
                current_time + poisson_timer.sample(&mut self.rng.lock().unwrap());
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

    pub fn get_inbounds_mut(&mut self) -> &mut HashMap<NodeId, Peer> {
        &mut self.in_peers
    }

    pub fn get_outbounds(&self) -> &HashMap<NodeId, Peer> {
        self.out_peers.inner()
    }

    pub fn get_outbounds_mut(&mut self) -> &mut HashMap<NodeId, Peer> {
        self.out_peers.inner_mut()
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

    /// Whether we know the simulated transaction
    pub fn knows_transaction(&self) -> bool {
        self.known_transaction
    }

    /// Whether the simulated transaction has already been requested to a peer
    pub fn has_requested_transaction(&self) -> bool {
        self.requested_transaction
    }

    /// Flag the simulated transaction as known
    fn add_known_transaction(&mut self) {
        assert!(!self.knows_transaction());
        self.known_transaction = true;
    }

    /// Whether the simulated transaction is already known or has already been requested
    fn is_transaction_known_or_requested(&self) -> bool {
        self.knows_transaction() || self.has_requested_transaction()
    }

    pub fn get_statistics(&self) -> &NodeStatistics {
        &self.node_statistics
    }

    /// Get the collection of peers selected to fanout the simulated transaction
    fn get_fanout_targets(&self) -> Vec<NodeId> {
        // Shortcut if we are not Erlay.
        // In theory, we would need to return the whole vector of peers, but this won't be used for non-erlay sims,
        // should_fanout_to would return true without checking the vector, so we can speed this up by just returning an empty vector
        if !self.is_erlay {
            return Vec::new();
        }

        let mut locked_rng = self.rng.lock().unwrap();
        let inbound_target_count = (self.get_inbounds().len() as f64
            * INBOUND_FANOUT_DESTINATIONS_FRACTION)
            .round() as usize;

        // In the real Bitcoin code, we perform a fancy ordering of the peers based on the transaction id that we want to send and the
        // peer_ids in our neighbourhood to obtain a deterministically random sorting from where we can pick our fanout peers.
        // In the simulator, we can make this way simpler, we only care about the ordering being deterministically random, and different
        // for multiples runs of the same simulation. This can be achieved by simply randomly sorting our peers using our pre-seeded rng.
        let mut targets = self
            .out_peers
            .keys()
            .copied()
            .choose_multiple(&mut *locked_rng, OUTBOUND_FANOUT_DESTINATIONS.into());

        targets.extend(
            self.in_peers
                .keys()
                .copied()
                .choose_multiple(&mut *locked_rng, inbound_target_count),
        );

        targets
    }

    /// Whether we should fanout the given transaction to the given peer.
    /// Assume that Erlay support is a boolean flag for all nodes in the simulation
    /// for now, so there are no fanout_tx_relay peers if Erlay is supported
    fn should_fanout_to(&self, peer_id: &NodeId, fanout_targets: &[NodeId]) -> bool {
        let peer = self.get_peer(peer_id).unwrap();
        if !peer.is_erlay() {
            return true;
        }

        fanout_targets.contains(peer_id)
    }

    /// Schedules a transaction announcement to be processed at a future time.
    /// We are initializing the transaction's originator interval sampling here. This is because we don't want to start
    /// sampling until we have something to send to our peers. Otherwise we would create useless events just for sampling.
    /// Notice this works since a Poisson process is memoryless hence past events do not affect future ones.
    fn schedule_tx_announcement(&mut self, current_time: u64) -> Vec<ScheduledEvent> {
        let mut events = Vec::new();

        // Collecting since we need to use them in the following loop, which also accesses self mutably
        let peers = self
            .in_peers
            .keys()
            .chain(self.out_peers.keys())
            .copied()
            .collect::<Vec<_>>();
        let fanout_targets = self.get_fanout_targets();

        for peer_id in peers {
            // Do not send the transaction to peers that already know about it (e.g. the peer that sent it to us)
            if !self.get_peer(&peer_id).unwrap().knows_transaction() {
                if self.should_fanout_to(&peer_id, &fanout_targets) {
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .add_tx_to_be_announced();
                } else {
                    // If this peer have been picked for set reconciliation, we can assume the tx_reconciliation_state is set
                    debug_log!(
                        current_time,
                        self.node_id,
                        "Added tx to reconciliation set for peer (peer_id: {peer_id})"
                    );
                    assert!(
                        self.get_peer_mut(&peer_id).unwrap().add_tx_to_reconcile(),
                        "Couldn't add tx to reconset (peer_id: {peer_id})",
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
            events.push(ScheduledEvent::new(
                Event::process_scheduled_announcement(self.node_id, peer_id),
                next_interval,
            ));
        }
        events
    }

    /// Kickstarts the broadcasting logic for the simulated transaction to all the node's peers. This includes both fanout and transaction
    /// reconciliation. For peers selected for fanout, an announcement is scheduled based on the peers type.
    /// Inbound peers use a shared Poisson timer with expected value of [INBOUND_INVENTORY_BROADCAST_INTERVAL] seconds,
    /// while outbound have a unique one with expected value of [OUTBOUND_INVENTORY_BROADCAST_INTERVAL] seconds.
    /// For peers selected for set reconciliation, this transaction is added to their reconciliation sets and will be made available
    /// when the next announcement is processed (this does not generate an event)
    /// Returns a collection of the scheduled events
    pub fn broadcast_tx(&mut self, current_time: u64) -> Vec<ScheduledEvent> {
        self.add_known_transaction();
        self.schedule_tx_announcement(current_time)
    }

    /// Reconciliation requests, as opposed to INVs, are sent on a schedule, no matter if we have transactions to reconcile or not
    /// (since our peer may have and we are unaware of this).
    /// Schedule the next reconciliation request.
    pub fn schedule_set_reconciliation(&mut self, request_time: u64) -> ScheduledEvent {
        // Make it so we reconcile with all peers every RECON_REQUEST_INTERVAL
        let delta = ((RECON_REQUEST_INTERVAL as f64 / self.out_peers.len() as f64)
            * SECS_TO_NANOS as f64)
            .round() as u64;
        ScheduledEvent::new(
            Event::process_scheduled_reconciliation(self.node_id),
            request_time + delta,
        )
    }

    /// Processes a previously scheduled reconciliation request. This starts the set reconciliation message exchange
    pub fn process_scheduled_reconciliation(
        &mut self,
        request_time: u64,
    ) -> (ScheduledEvent, ScheduledEvent) {
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
            .get_recon_set();

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

    /// Processes a previously scheduled transaction announcement to a given peer, returning an event to be scheduled if successful
    pub fn process_scheduled_announcement(
        &mut self,
        peer_id: NodeId,
        current_time: u64,
    ) -> Option<ScheduledEvent> {
        let peer = self.get_peer_mut(&peer_id).unwrap();

        // Make transactions that could have been announced via fanout available for reconciliation. Transactions added to the
        // reconciliation set between trickles are not available until the next interval
        if peer.is_erlay() {
            peer.get_tx_reconciliation_state_mut()
                .unwrap()
                .make_delayed_available();
        }

        // Drop the announcement if the peer has already learnt about the transaction during our delay
        // We are made aware of this by receiving an INV from the peer
        if peer.drain_tx_to_be_announced() && !peer.knows_transaction() {
            // We are simulating a single transaction, so that always fits within a single INV
            self.send_message_to(NetworkMessage::INV, peer_id, current_time)
        } else {
            None
        }
    }

    /// Records the request of the simulated transaction in or tracker (assigned to a given peer). If the peer is inbounds, this will generate
    /// a delayed request, and return an event to be processed at a future time. If the node is outbounds, the request will be generated
    /// straightaway.
    /// No request will be generated if we already know the transaction
    fn add_request(&mut self, peer_id: NodeId, request_time: u64) -> Option<ScheduledEvent> {
        if self.is_transaction_known_or_requested() {
            debug_log!(
                request_time,
                self.node_id,
                "Transactions is already known, or has already been requested, not requesting it to peer_id: {peer_id}",
            );
            return None;
        }

        // Transactions are only requested from a single peer (assuming honest behavior)
        // Inbound peers are de-prioritized. If an outbound peer announces a transaction
        // and an inbound peer request is in delayed stage, the inbounds will be dropped and
        // the outbound will be processed
        if self.is_peer_inbounds(&peer_id) {
            if self.delayed_request.is_none() {
                debug_log!(
                    request_time,
                    self.node_id,
                    "Delaying getdata to peer {peer_id}",
                );

                self.delayed_request = Some(peer_id);

                Some(ScheduledEvent::new(
                    Event::process_delayed_request(self.node_id, peer_id),
                    request_time + NONPREF_PEER_TX_DELAY * SECS_TO_NANOS,
                ))
            } else {
                debug_log!(
                    request_time,
                    self.node_id,
                    "There's already a pending delayed getdata request to peer {peer_id}, skipping",
                );
                None
            }
        } else {
            self.requested_transaction = true;
            Some(ScheduledEvent::new(
                Event::receive_message_from(self.node_id, peer_id, NetworkMessage::GETDATA),
                request_time,
            ))
        }
    }

    /// Processes a delayed request for a given inbound peer. If the transaction is still pending to be requested, the delayed request
    /// will be processed and an event will be generated. If the transactions have been already requested to outbound peers
    /// during our delay, the request will simply be dropped.
    /// Notice that we do not queue requests (because the nodes is the simulator are well behaved)
    pub fn process_delayed_request(
        &mut self,
        peer_id: NodeId,
        request_time: u64,
    ) -> Option<ScheduledEvent> {
        // If we are processing a delayed request we must have set it in the past
        assert!(self.delayed_request == Some(peer_id));
        self.delayed_request = None;

        if self.is_transaction_known_or_requested() {
            debug_log!(
                request_time,
                self.node_id,
                "A non-delayed request for the transaction has already been processed. Not requesting them to peer_id: {peer_id}",
            );
            return None;
        }

        self.requested_transaction = true;
        let msg = NetworkMessage::GETDATA;
        self.node_statistics.add_sent(&msg, true);

        Some(ScheduledEvent::new(
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
    ) -> Option<ScheduledEvent> {
        let message: Option<ScheduledEvent>;

        if let Some(peer) = self.get_peer(&peer_id) {
            match msg {
                NetworkMessage::INV => {
                    assert!(self.knows_transaction(), "Trying to announce the transaction to a peer (peer_id: {peer_id}), but we should't know about");
                    assert!(!peer.knows_transaction(), "Trying to announce a transaction to a peer that already knows about it (peer_id: {peer_id})");

                    message = Some(ScheduledEvent::new(
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ));
                }
                NetworkMessage::GETDATA => {
                    assert!(
                        !self.knows_transaction(),
                        "Trying to request the transaction from a peer (peer_id: {peer_id}), but we already know about it"
                    );
                    assert!(peer.knows_transaction(), "Trying to request a transaction from a peer that shouldn't know about it (peer_id: {peer_id})");

                    message = self.add_request(peer_id, request_time);
                }
                NetworkMessage::TX => {
                    assert!(self.knows_transaction(), "Trying to send the transaction to a peer (peer_id: {peer_id}), but we shouldn't know about it");
                    assert!(!peer.knows_transaction(), "Trying to send a transaction to a peer that already should know about it (peer_id: {peer_id})");
                    self.get_peer_mut(&peer_id).unwrap().add_known_transaction();

                    message = Some(ScheduledEvent::new(
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ));
                }
                NetworkMessage::REQRECON(has_tx) => {
                    assert!(self.is_erlay, "Trying to send a reconciliation request to peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(peer.is_erlay(), "Trying to send a reconciliation request to peer (peer_id: {peer_id}), but they do not support Erlay");
                    if has_tx {
                        assert!(self.knows_transaction(), "Trying to reconcile a transaction with a peer (peer_id: {peer_id}), but we should't know about");
                        // If we know that the peer knows, we have received at least an announcement from the peer (if not the transaction itself) meaning that the transaction shouldn't be
                        // in this peer's reconciliation set
                        assert!(!peer.knows_transaction(), "Trying to reconcile a transaction with a peer that already knows about it (peer_id: {peer_id})");
                    }
                    assert!(!peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Trying to send a reconciliation request to a peer we are already reconciling with (peer_id: {peer_id})");
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .get_tx_reconciliation_state_mut()
                        .unwrap()
                        .set_reconciling();
                    message = Some(ScheduledEvent::new(
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ))
                }
                NetworkMessage::SKETCH(sketch) => {
                    assert!(self.is_erlay, "Trying to send a reconciliation sketch to peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(peer.is_erlay(), "Trying to send a reconciliation sketch to peer (peer_id: {peer_id}), but they do not support Erlay");
                    assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Trying to send a reconciliation sketch to a peer that hasn't requested so (peer_id: {peer_id})");
                    // This check is a bit of a hack, but in the simulator short_ids and txids match
                    if sketch.get_tx_set() {
                        assert!(self.knows_transaction(), "Trying to send a sketch containing a transaction to a peer (peer_id: {peer_id}) but we should't know about it");
                    }
                    message = Some(ScheduledEvent::new(
                        Event::receive_message_from(self.node_id, peer_id, msg),
                        request_time,
                    ))
                }
                NetworkMessage::RECONCILDIFF(wants_tx) => {
                    assert!(self.is_erlay, "Trying to send a reconciliation difference to peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(peer.is_erlay(), "Trying to send a reconciliation difference to peer (peer_id: {peer_id}), but they do not support Erlay");
                    assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Trying to send a reconciliation difference to a peer that hasn't requested so (peer_id: {peer_id})");
                    if wants_tx {
                        assert!(!self.knows_transaction(), "Trying to request a transaction from a peer (peer_id: {peer_id}), but we should already know about");
                    }
                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .get_tx_reconciliation_state_mut()
                        .unwrap()
                        .clear(/*include_delayed=*/ false);
                    message = Some(ScheduledEvent::new(
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
        if let Some(event) = &message {
            if let Some(msg) = event.inner.get_message() {
                debug_log!(
                    request_time,
                    self.node_id,
                    "Sending {msg} to peer {peer_id}"
                );
                self.node_statistics
                    .add_sent(msg, self.is_peer_inbounds(&peer_id));
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
    ) -> Vec<ScheduledEvent> {
        let message: Vec<ScheduledEvent>;

        debug_log!(
            request_time,
            self.node_id,
            "Received {msg} from peer {peer_id}"
        );

        // We cannot hold a reference of peer here since we call mutable methods in the same context.
        // Maybe we can work around this by passing a reference to the peer instead of the node id
        // and getting another reference down the line
        if let Some(peer) = self.get_peer(&peer_id) {
            match msg {
                NetworkMessage::INV => {
                    self.get_peer_mut(&peer_id).unwrap().add_known_transaction();
                    // We only request transactions that we don't know about
                    if self.knows_transaction() {
                        debug_log!(request_time, self.node_id, "Already known transaction");
                        message = Vec::new()
                    } else {
                        message = self
                            .send_message_to(NetworkMessage::GETDATA, peer_id, request_time)
                            .map_or(Vec::new(), |x| vec![x])
                    }
                }
                NetworkMessage::GETDATA => {
                    assert!(self.knows_transaction(), "Received transaction request from a peer (peer_id: {peer_id}), but we don't know about the transaction");
                    assert!(!peer.knows_transaction(), "Received a transaction request from a peer that should already know about it (peer_id {peer_id})");
                    // Send tx cannot return None
                    message = self
                        .send_message_to(NetworkMessage::TX, peer_id, request_time)
                        .map_or(Vec::new(), |x| vec![x])
                }
                NetworkMessage::TX => {
                    assert!(!self.knows_transaction(), "Received the transaction from a peer (peer_id: {peer_id}), but we already knew about it");
                    assert!(peer.knows_transaction(), "Received a transaction from a node that shouldn't know about it (peer_id {peer_id})");
                    self.requested_transaction = true;
                    message = self.broadcast_tx(request_time)
                }
                NetworkMessage::REQRECON(has_tx) => {
                    assert!(self.is_erlay, "Received a reconciliation request from peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(peer.is_erlay(), "Received a reconciliation request from peer (peer_id: {peer_id}) but they do not support Erlay");
                    let peer = self.get_peer_mut(&peer_id).unwrap();
                    assert!(!peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Received a reconciliation request from a peer we are already reconciling with (peer_id: {peer_id})");
                    peer.get_tx_reconciliation_state_mut()
                        .unwrap()
                        .set_reconciling();
                    let sketch = peer
                        .get_tx_reconciliation_state()
                        .unwrap()
                        .compute_sketch(has_tx);

                    message = self
                        .send_message_to(NetworkMessage::SKETCH(sketch), peer_id, request_time)
                        .map_or(Vec::new(), |x| vec![x])
                }
                NetworkMessage::SKETCH(sketch) => {
                    assert!(self.is_erlay, "Received a reconciliation sketch from peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(peer.is_erlay(), "Received a reconciliation sketch from peer (peer_id: {peer_id}) but they do not support Erlay");
                    assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Received a reconciliation sketch from a peer that we haven't requested to reconcile with (peer_id: {peer_id})");

                    // Compute the local difference and remote difference between the sets (what we are missing and they are missing respectively)
                    let peer_recon_state = peer.get_tx_reconciliation_state().unwrap();
                    let (mut local_diff, remote_diff) =
                        peer_recon_state.compute_sketch_diff(sketch);

                    // Flag the peer as knowing the transaction  if we both know it. Any other of the two partial knowing cases
                    // (local_diff or remote_diff set, but not both) will be flagged either when we send them the transactions or when they
                    // send them to us
                    if !remote_diff && peer_recon_state.get_recon_set() {
                        self.get_peer_mut(&peer_id).unwrap().add_known_transaction();
                    }

                    // If we know the transaction but it is not in their recon set,  we picked this node for fanout.
                    // Flag peer as knowing and do not request it via set reconciliation
                    if local_diff && self.knows_transaction() {
                        local_diff = false;
                        self.get_peer_mut(&peer_id).unwrap().add_known_transaction();
                    }

                    // Send a RECONCILDIFF signaling whether we want the transaction or not, and an INV corresponding to the transaction if they are missing it
                    let mut events: Vec<ScheduledEvent> = self
                        .send_message_to(
                            NetworkMessage::RECONCILDIFF(local_diff),
                            peer_id,
                            request_time,
                        )
                        .map_or(Vec::new(), |x| vec![x]);

                    // If they are missing the transaction, send it via INV
                    if remote_diff {
                        assert!(!local_diff);
                        events.push(
                            self.send_message_to(NetworkMessage::INV, peer_id, request_time)
                                .unwrap(),
                        )
                    }

                    message = events
                }
                NetworkMessage::RECONCILDIFF(wants_tx) => {
                    let recon_state = peer.get_tx_reconciliation_state().unwrap();
                    assert!(self.is_erlay, "Received a reconciliation difference from peer (peer_id: {peer_id}) but we do not support Erlay");
                    assert!(peer.is_erlay(), "Received a reconciliation difference from peer (peer_id: {peer_id}) but they do not support Erlay");
                    assert!(peer.get_tx_reconciliation_state().unwrap().is_reconciling(), "Received a reconciliation difference from a peer that we haven't requested to reconcile with (peer_id: {peer_id})");
                    if wants_tx {
                        assert!(self.knows_transaction(), "Received a reconciliation difference from peer (peer_id: {peer_id}) containing a transaction we don't know about");
                        assert!(!peer.knows_transaction(), "Received a reconciliation difference from peer (peer_id: {peer_id}) containing a transaction they already know");
                    }

                    // If they don't want the transaction, and we have the it in their set, they already know it
                    if !wants_tx && recon_state.get_recon_set() {
                        let peer_mut = self.get_peer_mut(&peer_id).unwrap();
                        peer_mut.add_known_transaction()
                    }

                    self.get_peer_mut(&peer_id)
                        .unwrap()
                        .get_tx_reconciliation_state_mut()
                        .unwrap()
                        .clear(/*include_delayed=*/ false);

                    // Send them the transaction if they want it
                    if wants_tx {
                        message = self
                            .send_message_to(NetworkMessage::INV, peer_id, request_time)
                            .map_or(Vec::new(), |x| vec![x]);
                    } else {
                        message = Vec::new();
                    }
                }
            }
        } else {
            panic!("Received an message from a node we are not connected to (node_id: {peer_id})");
        }

        self.node_statistics
            .add_received(&msg, self.is_peer_inbounds(&peer_id));

        message
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
    fn test_add_known_tx() {
        let mut fanout_peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        fanout_peer.add_known_transaction();
        assert!(fanout_peer.knows_transaction());

        let mut erlay_peer = Peer::new(/*is_erlay=*/ true, /*is_inbound=*/ false);
        erlay_peer.add_known_transaction();
        assert!(erlay_peer.knows_transaction());

        // If an erlay peer has a transaction on their pending to be reconciled (either delayed or on the set)
        // and we add it to known, it will be removed from reconciliation
        erlay_peer.reset();
        erlay_peer.add_tx_to_reconcile();

        // The transaction is in its corresponding structures before flagging it as known
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set());

        erlay_peer
            .get_tx_reconciliation_state_mut()
            .unwrap()
            .make_delayed_available();

        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set());

        // And is removed after
        erlay_peer.add_known_transaction();
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());

        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
        assert!(erlay_peer.knows_transaction());
    }

    #[test]
    fn test_drain_txs_to_be_announced() {
        let mut peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);

        // Drain return false if there is no transaction in the set
        assert!(!peer.drain_tx_to_be_announced());

        // And true if there is, but then removes it
        peer.add_tx_to_be_announced();
        assert!(peer.drain_tx_to_be_announced());
        assert!(!peer.to_be_announced);
    }

    #[test]
    fn test_add_tx_to_reconcile() {
        // Fanout peers have no reconciliation state, so data cannot be added to it
        let mut fanout_peer = Peer::new(/*is_erlay=*/ false, /*is_inbound=*/ false);
        assert!(!fanout_peer.add_tx_to_reconcile());

        // Erlay peers do have reconciliation state, independently of whether they are initiators or not. Data added
        // to the set is put on the delayed collection first, and moved to the actual set on demand
        let mut erlay_peer = Peer::new(/*is_erlay=*/ true, /*is_inbound=*/ false);
        assert!(erlay_peer.add_tx_to_reconcile());
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set());
        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());

        // Make transactions available for reconciliation. This usually happens on the next trickle interval for the peer
        erlay_peer
            .get_tx_reconciliation_state_mut()
            .unwrap()
            .make_delayed_available();

        assert!(!erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set());
        assert!(erlay_peer
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
    }
}

#[cfg(test)]
mod test_node {

    use super::*;
    use rand::SeedableRng;

    #[test]
    fn test_get_next_announcement_time() {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
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
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
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
    fn test_schedule_tx_announcement_no_erlay() {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);

        let outbound_peer_ids = Vec::from_iter(1..11);
        for peer_id in outbound_peer_ids.iter() {
            node.connect(*peer_id, false);
        }

        // For non-erlay peers, schedule_tx_announcement creates an event for each peer
        // This is completely independent of whether the peer is inbound or outbound
        let events = node.schedule_tx_announcement(0);
        assert_eq!(events.len(), outbound_peer_ids.len());
        for e in events {
            assert!(matches!(e.inner, Event::ProcessScheduledAnnouncement(..)));
            if let Event::ProcessScheduledAnnouncement(src, dst) = e.inner {
                assert_eq!(src, node_id);
                assert!(outbound_peer_ids.contains(&dst));
                assert!(node.out_peers.get(&dst).unwrap().to_be_announced)
            };
        }
    }

    #[test]
    fn test_schedule_tx_announcement_erlay() {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
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
        let events = node.schedule_tx_announcement(current_time);
        assert_eq!(events.len(), outbound_peer_ids.len());
        for e in events {
            assert!(matches!(e.inner, Event::ProcessScheduledAnnouncement(..)));
            // Events are all in the future
            assert!(e.time() > current_time);
            if let Event::ProcessScheduledAnnouncement(src, dst) = e.inner {
                assert_eq!(src, node_id);
                assert!(outbound_peer_ids.contains(&dst));
                // The transaction is added to to_be_announced or to the recon_set depending on should_fanout_to
                if node.out_peers.get(&dst).unwrap().to_be_announced {
                    assert!(!node
                        .out_peers
                        .get(&dst)
                        .unwrap()
                        .get_tx_reconciliation_state()
                        .unwrap()
                        .get_delayed_set());
                    fanout_count += 1;
                } else {
                    assert!(node
                        .out_peers
                        .get(&dst)
                        .unwrap()
                        .get_tx_reconciliation_state()
                        .unwrap()
                        .get_delayed_set());
                    reconciliation_count += 1;
                }
            };
        }

        // With 10 outbound peers, one should be picked as fanout, and the rest as set recon
        assert_eq!(fanout_count, OUTBOUND_FANOUT_DESTINATIONS);
        assert_eq!(reconciliation_count, 10 - OUTBOUND_FANOUT_DESTINATIONS);
    }

    #[test]
    fn test_broadcast_tx() {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
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

        let events = node.broadcast_tx(0);
        assert!(node.knows_transaction());

        for e in events {
            // Events are all in the future
            assert!(e.time() > current_time);
            assert!(matches!(e.inner, Event::ProcessScheduledAnnouncement(..)));
            if let Event::ProcessScheduledAnnouncement(src, dst) = e.inner {
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
            let mut node = Node::new(
                node_id,
                Arc::new(Mutex::new(StdRng::from_entropy())),
                true,
                true,
            );
            // Connect the desired amount of nodes
            let outbound_peer_ids = Vec::from_iter(0..*peers_size);
            for peer_id in outbound_peer_ids.into_iter() {
                node.connect(peer_id, true);
            }

            // Check the schedule (we use request_time=0 for simplicity)
            let e = node.schedule_set_reconciliation(current_time);
            assert!(matches!(e.inner, Event::ProcessScheduledReconciliation(..)));
            assert_eq!(
                e.time(),
                (RECON_REQUEST_INTERVAL / *peers_size as u64) * SECS_TO_NANOS
            )
        }
    }

    #[test]
    fn test_process_scheduled_reconciliation() {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let outbound_peer_ids = Vec::from_iter(1..11);
        let current_time = 0;

        node.add_known_transaction();

        for peer_id in outbound_peer_ids.iter() {
            node.connect(*peer_id, true);

            // Add a transaction to the reconciliation set of the peer (skip delayed)
            let recon_set_mut = node
                .get_peer_mut(peer_id)
                .unwrap()
                .get_tx_reconciliation_state_mut()
                .unwrap();
            recon_set_mut.add_tx();
            recon_set_mut.make_delayed_available();
        }

        // Iterating over again to process the reconciliations because we want peers
        // to be connected already
        for _ in outbound_peer_ids.iter() {
            let (req_recon, scheduled_recon) = node.process_scheduled_reconciliation(current_time);

            // Check that we receive the two events we are expecting, and that reconciliation request (former event)
            // contains the transaction
            assert!(matches!(req_recon.inner, Event::ReceiveMessageFrom(..)));
            assert!(matches!(
                req_recon.inner.get_message().unwrap(),
                NetworkMessage::REQRECON(..)
            ));
            if let NetworkMessage::REQRECON(has_tx) = req_recon.inner.get_message().unwrap() {
                assert!(has_tx);
            }
            assert!(matches!(
                scheduled_recon.inner,
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
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let current_time = 0;

        // We won't need more than one peer to test this
        let peer_id_fanout = 1;
        let peer_id_recon = 2;
        node.connect(peer_id_fanout, true);
        node.connect(peer_id_recon, true);

        // Processing a scheduled announcement with no data to be sent returns nothing
        assert!(node
            .process_scheduled_announcement(peer_id_fanout, current_time)
            .is_none());

        // If the peer has data pending to be sent, an INV message containing such data will be returned.
        // Also, if the peer had some data to be reconciled, that data will be made available (moved out of the delayed set)

        // Add the transaction as to be announced for one, and reconciled for the other
        node.add_known_transaction();
        node.get_peer_mut(&peer_id_fanout)
            .unwrap()
            .add_tx_to_be_announced();
        node.get_peer_mut(&peer_id_recon)
            .unwrap()
            .add_tx_to_reconcile();

        // The transaction to be reconciled is delayed
        assert!(!node
            .get_peer(&peer_id_recon)
            .unwrap()
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
        assert!(node
            .get_peer(&peer_id_recon)
            .unwrap()
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set(),);

        // Professing the scheduled announcement returns an INV (meaning that the transaction is known and processed)
        let inv_event = node
            .process_scheduled_announcement(peer_id_fanout, current_time)
            .unwrap();
        assert!(matches!(inv_event.inner, Event::ReceiveMessageFrom(..)));
        assert!(inv_event.inner.get_message().unwrap().is_inv());

        // Processing the scheduled announcement for the erlay peer moves the transaction to available
        // No INV is returned in this case
        assert!(node
            .process_scheduled_announcement(peer_id_recon, current_time)
            .is_none());

        assert!(!node
            .get_peer(&peer_id_recon)
            .unwrap()
            .get_tx_reconciliation_state()
            .unwrap()
            .get_delayed_set());
        assert!(node
            .get_peer(&peer_id_recon)
            .unwrap()
            .get_tx_reconciliation_state()
            .unwrap()
            .get_recon_set());
    }

    #[test]
    fn test_add_request() {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let current_time = 0;

        // Add one outbound and one inbound peer
        let outbound_id = 1;
        let inbound_id = 2;
        node.connect(outbound_id, true);
        node.accept_connection(inbound_id, true);

        // Adding a request for a transaction we don't know will generate a get data message
        let e = node.add_request(outbound_id, current_time).unwrap();
        assert_eq!(e.time(), current_time);
        assert!(e.inner.is_receive_message());
        if let Event::ReceiveMessageFrom(s, d, m) = e.inner {
            assert_eq!(s, node_id);
            assert_eq!(d, outbound_id);
            assert!(m.is_get_data())
        }

        assert!(node.has_requested_transaction());

        // Trying to add a request twice won't generate a get data again
        assert!(node.add_request(outbound_id, current_time).is_none());
        // This holds even if we try to request it to another peer
        assert!(node.add_request(inbound_id, current_time).is_none());

        // If the peer is inbound instead of outbounds, the request is delayed instead of processed straightaway
        node.requested_transaction = false; // We need to reset this manually since the transaction can only be requested once
        assert!(node.delayed_request.is_none());
        let e = node.add_request(inbound_id, current_time).unwrap();
        assert_eq!(e.inner, Event::ProcessDelayedRequest(node_id, inbound_id));
        assert!(e.time() > current_time);
        // The transaction is kept in the delayed_requests collection for future processing
        assert!(node.delayed_request.is_some());
    }

    #[test]
    fn test_process_delayed_request() {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
        let node_id = 0;
        let mut node = Node::new(node_id, rng, true, true);
        let current_time = 0;

        let inbound_id = 1;
        node.accept_connection(inbound_id, true);

        // Add a delayed request
        node.add_request(inbound_id, current_time);
        assert!(node.delayed_request.is_some());
        // Process the delayed request
        let e = node
            .process_delayed_request(inbound_id, current_time)
            .unwrap();
        assert!(node.delayed_request.is_none());
        assert!(e.inner.is_receive_message());
        if let Event::ReceiveMessageFrom(s, d, m) = e.inner {
            assert_eq!(s, node_id);
            assert_eq!(d, inbound_id);
            assert!(m.is_get_data());
        }
        assert!(node.has_requested_transaction());

        // If the transaction is already known (because a non-delayed request containing it was already processed)
        // there won't be any event
        assert!(node.add_request(inbound_id, current_time).is_none());
    }

    // [Node::send_message_to] and [Node::receive_message_from] are self tested.
    // They include asserts for every sent/received message that would make the
    // message flow fail on runtime if anything was wrong
}
