#[cfg(feature = "graph")]
use graphrs::Graph;
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use rand::rngs::StdRng;
use rand::{thread_rng, Rng, RngCore, SeedableRng};

#[cfg(feature = "graph")]
use crate::graph::NodeAttributes;
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
    /// Processes a scheduled reconciliation on the given node
    /// Peers are reconciled on a round robin fashion, so there is
    /// no need to specify the target
    ProcessScheduledReconciliation(NodeId),
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

    pub fn process_scheduled_reconciliation(src: NodeId) -> Self {
        Event::ProcessScheduledReconciliation(src)
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
            Event::ProcessScheduledReconciliation(_) => None,
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
    rng: Arc<Mutex<StdRng>>,
    /// The simulated network
    pub network: Network,
    /// A queue of the events that make the simulation, ordered by discrete time
    event_queue: BinaryHeap<ScheduledEvent>,
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
        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed.unwrap())));
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
        }
    }

    // Creates a simulator from a given [Network]. Useful when creating simulations using the graph feature
    #[cfg(feature = "graph")]
    pub fn from_graph(graph: Graph<NodeId, NodeAttributes>, seed: &mut Option<u64>) -> Self {
        if let Some(s) = seed {
            log::info!("Using user provided rng seed: {}", s);
        } else {
            *seed = Some(thread_rng().next_u64());
            log::info!("Using fresh rng seed: {}", seed.unwrap());
        };

        let rng = Arc::new(Mutex::new(StdRng::seed_from_u64(seed.unwrap())));
        Self {
            rng: rng.clone(),
            network: (graph, rng.clone()).into(),
            event_queue: BinaryHeap::new(),
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
                let delta = self
                    .rng
                    .lock()
                    .unwrap()
                    .gen_range(0..RECON_REQUEST_INTERVAL * SECS_TO_NANOS);
                self.event_queue
                    .push(node.schedule_set_reconciliation(current_time + delta));
            }
        }
    }

    /// Adds an event to the event queue, adding random latency if the event is [Event::ReceiveMessageFrom].
    /// These latencies simulate the messages traveling across the network
    pub fn add_event(&mut self, mut scheduled_event: ScheduledEvent) {
        let event = &scheduled_event.inner;
        if event.is_receive_message() {
            if let Some(link) = event.get_link() {
                let latency = *self.network.get_links().get(&link).unwrap_or_else(|| {
                    panic!(
                        "No connection found between node: {} and node {}",
                        link.a(),
                        link.b(),
                    )
                });
                scheduled_event.time.0 += latency;
            }
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
        self.rng
            .lock()
            .unwrap()
            .gen_range(0..self.network.get_node_count())
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
