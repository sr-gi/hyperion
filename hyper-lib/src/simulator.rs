use std::cmp::Reverse;
use std::hash::Hash;

use priority_queue::PriorityQueue;
use rand::rngs::StdRng;
use rand::{thread_rng, Rng, RngCore, SeedableRng};
use rand_distr::{Distribution, LogNormal};

use crate::network::{Network, NetworkMessage};
use crate::node::{Node, NodeId};
use crate::{TxId, SECS_TO_NANOS};

static NET_LATENCY_MEAN: f64 = 0.01 * SECS_TO_NANOS as f64; // 10ms

/// An enumeration of all the events that can be created in a simulation
#[derive(Clone, Hash, Eq, PartialEq)]
pub enum Event {
    /// The destination (0) receives a new message (2) from given source (1)
    ReceiveMessageFrom(NodeId, NodeId, NetworkMessage),
    /// A given node (0) processes an scheduled announcements to a given peer (1)
    ProcessScheduledAnnouncement(NodeId, NodeId),
    /// A given node (0) processed a delayed request of a give transaction (1)
    ProcessDelayedRequest(NodeId, NodeId),
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

    pub fn is_receive_message(&self) -> bool {
        matches!(self, Event::ReceiveMessageFrom(..))
    }

    pub fn get_message(&self) -> Option<&NetworkMessage> {
        match self {
            Event::ReceiveMessageFrom(_, _, m) => Some(m),
            _ => None,
        }
    }
}

pub struct Simulator {
    /// A pre-seeded rng to allow reproducing previous simulation results
    rng: StdRng,
    /// A function used to generate network latency. Follows a LogNormal distribution over a given value
    net_latency_fn: Option<LogNormal<f64>>,
    /// The simulated network
    pub network: Network,
    /// A queue of the events that make the simulation, ordered by discrete time
    event_queue: PriorityQueue<Event, Reverse<u64>>,
}

impl Simulator {
    pub fn new(
        reachable_count: usize,
        unreachable_count: usize,
        is_erlay: bool,
        seed: Option<u64>,
        network_latency: bool,
    ) -> Self {
        let seed = if let Some(seed) = seed {
            log::info!("Using user provided rng seed: {}", seed);
            seed
        } else {
            let s = thread_rng().next_u64();
            log::info!("Using fresh rng seed: {}", s);
            s
        };
        let mut rng: StdRng = StdRng::seed_from_u64(seed);
        let network = Network::new(reachable_count, unreachable_count, is_erlay, &mut rng);

        // Create a network latency function for sent/received messages. This is in the order of
        // nanoseconds, using a LogNormal distribution with expected value NET_LATENCY_MEAN, and
        // variance of 20% of the expected value
        let net_latency_fn = if network_latency {
            Some(LogNormal::from_mean_cv(NET_LATENCY_MEAN, 0.2).unwrap())
        } else {
            None
        };

        Self {
            rng,
            net_latency_fn,
            network,
            event_queue: PriorityQueue::new(),
        }
    }

    /// Adds an event to the event queue, adding random latency if the event is [Event::ReceiveMessageFrom].
    /// These latencies simulate the messages traveling across the network
    pub fn add_event(&mut self, event: Event, mut time: u64) {
        if self.net_latency_fn.is_some() && event.is_receive_message() {
            time += self.net_latency_fn.unwrap().sample(&mut self.rng).round() as u64;
        }

        self.event_queue.push(event, Reverse(time));
    }

    /// Get the next event to be processed, as in the one with the smallest discrete time
    pub fn get_next_event(&mut self) -> Option<(Event, u64)> {
        self.event_queue.pop().map(|(e, t)| (e, t.0))
    }

    pub fn get_random_txid(&mut self) -> TxId {
        self.rng.next_u32()
    }

    pub fn get_random_nodeid(&mut self) -> NodeId {
        self.rng.gen_range(0..self.network.get_node_count())
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
