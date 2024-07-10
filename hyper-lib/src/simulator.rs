use std::cmp::Reverse;
use std::hash::Hash;

use priority_queue::PriorityQueue;
use rand::rngs::StdRng;
use rand::{thread_rng, Rng, RngCore, SeedableRng};

use crate::network::{Network, NetworkMessage};
use crate::node::{Node, NodeId};
use crate::TxId;

#[derive(Clone, Hash, Eq, PartialEq)]
pub enum Event {
    SampleNewInterval(NodeId, Option<NodeId>),
    ReceiveMessageFrom(NodeId, NodeId, NetworkMessage),
    ProcessDelayedRequest(NodeId, TxId),
}

impl Event {
    pub fn sample_new_interval(target: NodeId, peer_id: Option<NodeId>) -> Self {
        Event::SampleNewInterval(target, peer_id)
    }

    pub fn receive_message_from(src: NodeId, dst: NodeId, msg: NetworkMessage) -> Self {
        Event::ReceiveMessageFrom(src, dst, msg)
    }

    pub fn process_delayed_request(src: NodeId, txid: TxId) -> Self {
        Event::ProcessDelayedRequest(src, txid)
    }

    pub fn is_delayed_request(&self) -> bool {
        matches!(self, Event::ProcessDelayedRequest(..))
    }
}

pub struct Simulator {
    pub rng: StdRng,
    pub network: Network,
    pub event_queue: PriorityQueue<Event, Reverse<u64>>,
}

impl Simulator {
    pub fn new(reachable_count: usize, unreachable_count: usize) -> Self {
        let mut rng: StdRng = StdRng::seed_from_u64(thread_rng().next_u64());
        let network = Network::new(reachable_count, unreachable_count, &mut rng);

        Self {
            rng,
            network,
            event_queue: PriorityQueue::new(),
        }
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
