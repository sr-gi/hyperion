use std::cell::RefCell;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::hash::Hash;
use std::rc::Rc;

use rand::rngs::StdRng;
use rand::{thread_rng, Rng, RngCore, SeedableRng};

use crate::network::{Link, Network, NetworkMessage};
use crate::node::{Node, NodeId, RECON_REQUEST_INTERVAL};
use crate::SECS_TO_NANOS;

/// An enumeration of all the events that can be created in a simulation
#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub enum Event {
    /// The destination (0) receives a new message (2) from given source (1)
    ReceiveMessageFrom(NodeId, NodeId, NetworkMessage),
    /// A given node (0) processes an scheduled announcements to a given peer (1)
    ProcessScheduledAnnouncement(NodeId, NodeId),
    /// A given node (0) processed a delayed request of a give transaction (1)
    ProcessDelayedRequest(NodeId, NodeId),
    /// Processes a scheduled reconciliation on the given node (0) with a given peer (1)
    ProcessScheduledReconciliation(NodeId, NodeId),
}

impl Event {
    pub fn receive_message_from(src: NodeId, dst: NodeId, msg: NetworkMessage) -> Self {
        Event::ReceiveMessageFrom(src, dst, msg)
    }

    pub fn process_scheduled_announcement(src: NodeId, dst: NodeId) -> Self {
        Event::ProcessScheduledAnnouncement(src, dst)
    }

    pub fn process_delayed_request(src: NodeId, dst: NodeId) -> Self {
        Event::ProcessDelayedRequest(src, dst)
    }

    pub fn process_scheduled_reconciliation(src: NodeId, dst: NodeId) -> Self {
        Event::ProcessScheduledReconciliation(src, dst)
    }

    pub fn is_receive_message(&self) -> bool {
        matches!(self, Event::ReceiveMessageFrom(..))
    }

    pub fn get_message(&self) -> Option<&NetworkMessage> {
        match self {
            Event::ReceiveMessageFrom(_, _, m) => Some(m),
            _ => None,
        }
    }

    pub fn get_link(&self) -> Option<Link> {
        match self {
            Event::ReceiveMessageFrom(a, b, _) => Some((*a, *b).into()),
            Event::ProcessScheduledAnnouncement(a, b) => Some((*a, *b).into()),
            Event::ProcessDelayedRequest(a, b) => Some((*a, *b).into()),
            Event::ProcessScheduledReconciliation(a, b) => Some((*a, *b).into()),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ScheduledEvent {
    pub inner: Event,
    time: Reverse<u64>,
}

impl ScheduledEvent {
    pub fn new(event: Event, time: u64) -> Self {
        ScheduledEvent {
            inner: event,
            time: Reverse(time),
        }
    }

    pub fn time(&self) -> u64 {
        self.time.0
    }
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time.cmp(&other.time)
    }
}

// `PartialOrd` needs to be implemented as well.
impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<ScheduledEvent> for (Event, u64) {
    fn from(event: ScheduledEvent) -> Self {
        let t = event.time();
        (event.inner, t)
    }
}

pub struct Simulator {
    /// A pre-seeded rng to allow reproducing previous simulation results
    rng: Rc<RefCell<StdRng>>,
    /// The simulated network
    pub network: Network,
    /// A queue of the events that make the simulation, ordered by discrete time
    event_queue: BinaryHeap<ScheduledEvent>,
    /// A cached random node identifier, initialized before the network is computed and
    /// re-written every time get_random_nodeid is called
    cached_node_id: NodeId,
}

impl Simulator {
    pub fn new(
        reachable_count: usize,
        unreachable_count: usize,
        outbounds_count: usize,
        is_erlay: bool,
        seed: &mut Option<u64>,
        network_latency: bool,
    ) -> Self {
        if let Some(s) = seed {
            log::info!("Using user provided rng seed: {}", s);
        } else {
            *seed = Some(thread_rng().next_u64());
            log::info!("Using fresh rng seed: {}", seed.unwrap());
        };
        let rng = Rc::new(RefCell::new(StdRng::seed_from_u64(seed.unwrap())));
        let random_node_id = rng
            .borrow_mut()
            .gen_range(0..reachable_count + unreachable_count);

        let network = Network::new(
            reachable_count,
            unreachable_count,
            outbounds_count,
            network_latency,
            is_erlay,
            rng.clone(),
        );

        Self {
            rng,
            network,
            event_queue: BinaryHeap::new(),
            cached_node_id: random_node_id,
        }
    }

    pub fn schedule_set_reconciliation(&mut self, current_time: u64) {
        if self.network.is_erlay() {
            for node in self.network.get_nodes_mut() {
                // Schedule transaction reconciliation here. As opposite to fanout, reconciliation is scheduled
                // on a fixed interval. This means that we need to start it when the connection is made. However,
                // in the simulator, the whole network is build at the same (discrete) time. This does not follow
                // reality, so we will pick a random value between the simulation start time (current_time) and
                // RECON_REQUEST_INTERVAL as the first scheduled reconciliation for each connection.
                let start_time = current_time
                    + self
                        .rng
                        .borrow_mut()
                        .gen_range(0..RECON_REQUEST_INTERVAL * SECS_TO_NANOS);

                // Make it so we reconcile with all peers every RECON_REQUEST_INTERVAL
                let outbound_peers = node.get_outbounds();
                let delta = ((RECON_REQUEST_INTERVAL as f64 / outbound_peers.len() as f64)
                    * SECS_TO_NANOS as f64)
                    .round() as u64;

                for (i, peer_id) in outbound_peers.keys().enumerate() {
                    // Schedule interleaved reconciliation. All outbound peers are reconciled every RECON_REQUEST_INTERVAL, with a
                    // RECON_REQUEST_INTERVAL/N step, where N is the number of outbound peers
                    self.event_queue.push(
                        node.schedule_set_reconciliation(peer_id, start_time + (delta * i as u64)),
                    );
                }
            }
        }
    }

    /// Adds an event to the event queue, adding random latency if the event is [Event::ReceiveMessageFrom].
    /// These latencies simulate the messages traveling across the network
    pub fn add_event(&mut self, mut scheduled_event: ScheduledEvent) {
        let event = &scheduled_event.inner;
        if event.is_receive_message() && self.network.has_latency() {
            let link = &event.get_link().unwrap();
            let latency = *self.network.get_links().get(link).unwrap_or_else(|| {
                panic!(
                    "No connection found between node: {} and node {}",
                    link.a(),
                    link.b(),
                )
            });
            scheduled_event.time.0 += latency;
        }

        self.event_queue.push(scheduled_event);
    }

    /// Get the next event to be processed, as in the one with the smallest discrete time
    pub fn get_next_event(&mut self) -> Option<ScheduledEvent> {
        self.event_queue.pop()
    }

    /// Get the time when the next event will be processed
    pub fn get_next_event_time(&mut self) -> Option<u64> {
        let scheduled_event = self.event_queue.peek()?;
        Some(scheduled_event.time())
    }

    pub fn get_random_nodeid(&mut self) -> NodeId {
        let random_node_id = self.cached_node_id;
        self.cached_node_id = self
            .rng
            .borrow_mut()
            .gen_range(0..self.network.get_node_count());
        random_node_id
    }

    pub fn get_node(&self, node_id: NodeId) -> Option<&Node> {
        self.network.get_node(node_id)
    }

    pub fn get_node_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.network.get_node_mut(node_id)
    }

    pub fn get_nodes(&self) -> &Vec<Node> {
        self.network.get_nodes()
    }
}
