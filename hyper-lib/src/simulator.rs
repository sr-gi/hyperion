use std::cmp::Reverse;
use std::hash::Hash;

use priority_queue::PriorityQueue;

use crate::network::NetworkMessage;
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
    pub network: Vec<Node>,
    pub event_queue: PriorityQueue<Event, Reverse<u64>>,
}

impl Simulator {
    pub fn new(node_count: usize) -> Self {
        Self {
            network: Vec::with_capacity(node_count),
            event_queue: PriorityQueue::new(),
        }
    }
}
