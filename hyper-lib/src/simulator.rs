use std::cmp::Reverse;
use std::hash::Hash;

use priority_queue::PriorityQueue;
use rand::rngs::StdRng;
use rand::{thread_rng, Rng, RngCore, SeedableRng};
use rand_distr::{Distribution, LogNormal};

use crate::network::{Network, NetworkMessage};
use crate::node::{Node, NodeId};
use crate::{TxId, SECS_TO_NANOS};

static NET_DELAY_MEAN: f64 = 0.01 * SECS_TO_NANOS as f64; // 10ms

#[derive(Clone, Hash, Eq, PartialEq)]
pub enum Event {
    SampleNewInterval(NodeId, Option<NodeId>),
    ReceiveMessageFrom(NodeId, NodeId, NetworkMessage),
    ProcessScheduledAnnouncement(NodeId, NodeId, TxId),
    ProcessDelayedRequest(NodeId, TxId),
}

impl Event {
    pub fn sample_new_interval(target: NodeId, peer_id: Option<NodeId>) -> Self {
        Event::SampleNewInterval(target, peer_id)
    }

    pub fn receive_message_from(src: NodeId, dst: NodeId, msg: NetworkMessage) -> Self {
        Event::ReceiveMessageFrom(src, dst, msg)
    }

    pub fn process_delayed_announcement(src: NodeId, dst: NodeId, txid: TxId) -> Self {
        Event::ProcessScheduledAnnouncement(src, dst, txid)
    }

    pub fn process_delayed_request(src: NodeId, txid: TxId) -> Self {
        Event::ProcessDelayedRequest(src, txid)
    }

    pub fn is_receive_message(&self) -> bool {
        matches!(self, Event::ReceiveMessageFrom(..))
    }

    pub fn is_delayed_request(&self) -> bool {
        matches!(self, Event::ProcessDelayedRequest(..))
    }
}

pub struct Simulator {
    rng: StdRng,
    net_delay_fn: LogNormal<f64>,
    pub network: Network,
    event_queue: PriorityQueue<Event, Reverse<u64>>,
}

impl Simulator {
    pub fn new(reachable_count: usize, unreachable_count: usize, seed: Option<u64>) -> Self {
        let seed = if let Some(seed) = seed {
            log::info!("Using user provided rng seed: {}", seed);
            seed
        } else {
            let s = thread_rng().next_u64();
            log::info!("Using fresh rng seed: {}", s);
            s
        };
        let mut rng: StdRng = StdRng::seed_from_u64(seed);
        let network = Network::new(reachable_count, unreachable_count, &mut rng);

        // Create a network delay function for sent/received messages. This is in the order of
        // nanoseconds, using a LogNormal distribution with expected value NET_DELAY_MEAN, and
        // variance NET_DELAY_MEAN/5
        let net_delay_fn: LogNormal<f64> =
            LogNormal::from_mean_cv(NET_DELAY_MEAN, NET_DELAY_MEAN * 0.2).unwrap();

        Self {
            rng,
            net_delay_fn,
            network,
            event_queue: PriorityQueue::new(),
        }
    }

    pub fn add_event(&mut self, event: Event, mut time: u64) {
        if event.is_receive_message() {
            time += self.net_delay_fn.sample(&mut self.rng).round() as u64;
        }

        self.event_queue.push(event, Reverse(time));
    }

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
