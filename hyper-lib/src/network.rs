use std::sync::mpsc::Receiver;

use crate::node::{Node, NodeId};
use crate::TxId;

#[derive(Copy, Clone)]
pub enum NetworkMessage {
    INV(TxId),
    GETDATA(TxId),
    TX(TxId),
}

impl NetworkMessage {
    pub fn inner(&self) -> &TxId {
        match self {
            NetworkMessage::INV(x) => &x,
            NetworkMessage::GETDATA(x) => &x,
            NetworkMessage::TX(x) => &x,
        }
    }

    fn get_size(&self) -> u32 {
        // FIXME: Add some sizes that make sense
        match self {
            NetworkMessage::INV(_) => 32,
            NetworkMessage::GETDATA(_) => 32,
            NetworkMessage::TX(_) => 150,
        }
    }

    pub fn is_inv(&self) -> bool {
        matches!(self, NetworkMessage::INV(..))
    }

    pub fn is_get_data(&self) -> bool {
        matches!(self, NetworkMessage::GETDATA(..))
    }

    pub fn is_tx(&self) -> bool {
        matches!(self, NetworkMessage::TX(..))
    }
}

impl std::fmt::Display for NetworkMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (m, txid) = match self {
            NetworkMessage::INV(x) => ("inv", x),
            NetworkMessage::GETDATA(x) => ("getdata", x),
            NetworkMessage::TX(x) => ("tx", x),
        };

        write!(f, "{m}(txid: {txid:x})")
    }
}

pub struct Network {
    pub nodes: Vec<Node>,
    pub messages: Receiver<(NetworkMessage, NodeId, NodeId)>,
}

impl Network {
    pub fn new(node_count: usize, messages: Receiver<(NetworkMessage, NodeId, NodeId)>) -> Self {
        Self {
            nodes: Vec::with_capacity(node_count),
            messages,
        }
    }
}
