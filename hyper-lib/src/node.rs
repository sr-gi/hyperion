use std::collections::{HashMap, HashSet, VecDeque};

pub type NodeId = u32;
pub type TxId = u32;

#[derive(Clone)]
pub enum NetworkMessage {
    INV(TxId),
    GETDATA(TxId),
    TX(TxId),
}

impl NetworkMessage {
    fn inner(&self) -> &TxId {
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

    fn is_inv(&self) -> bool {
        matches!(self, NetworkMessage::INV(..))
    }

    fn is_get_data(&self) -> bool {
        matches!(self, NetworkMessage::GETDATA(..))
    }

    fn is_tx(&self) -> bool {
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

#[derive(Clone)]
pub struct Peer {
    known_transactions: HashSet<TxId>,
    message_queue: VecDeque<NetworkMessage>,
}

impl Peer {
    pub fn new() -> Self {
        Peer {
            known_transactions: HashSet::new(),
            message_queue: VecDeque::new(),
        }
    }

    fn knows_transactions(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    fn enqueue_message(&mut self, msg: NetworkMessage) {
        self.message_queue.push_back(msg)
    }
}

#[derive(Clone)]
pub struct Node {
    node_id: NodeId,
    is_reachable: bool,
    in_peers: HashMap<NodeId, Peer>,
    out_peers: HashMap<NodeId, Peer>,
    known_transactions: HashSet<TxId>,
}

impl Node {
    pub fn new(node_id: NodeId, is_reachable: bool) -> Self {
        Node {
            node_id,
            is_reachable,
            in_peers: HashMap::new(),
            out_peers: HashMap::new(),
            known_transactions: HashSet::new(),
        }
    }

    fn get_peer(&self, peer_id: NodeId) -> Option<&Peer> {
        self.out_peers
            .get(&peer_id)
            .or_else(|| self.in_peers.get(&peer_id))
    }

    fn get_peer_mut(&mut self, peer_id: NodeId) -> Option<&mut Peer> {
        self.out_peers
            .get_mut(&peer_id)
            .or_else(|| self.in_peers.get_mut(&peer_id))
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
    }

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
            self.in_peers.insert(peer_id, Peer::new()).is_none(),
            "Peer {peer_id} is already connected to us"
        );
    }

    fn knows_transactions(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    pub fn send_message_to(&mut self, msg: NetworkMessage, peer_id: NodeId) {
        if let Some(peer) = self.get_peer_mut(peer_id) {
            let txid = *msg.inner();
            if msg.is_inv() {
                // INVs for the same transaction can get crossed. If this happens break the cycle.
                if peer.knows_transactions(&txid) {
                    return;
                }
            } else if msg.is_get_data() {
                assert!(peer.knows_transactions(&txid), "Trying to request a transaction (txid: {txid}) from a peers that doesn't know about it (peer_id: {peer_id})");
            } else {
                assert!(!peer.knows_transactions(&txid), "Trying to send a transaction (txid: {txid}) from a peers that already knows about it (peer_id: {peer_id})");
                peer.add_known_transaction(txid);
            }
            log::info!("Sending {msg} to peer {peer_id}");
            peer.enqueue_message(msg);
        }
    }

    pub fn receive_message_from(&mut self, msg: NetworkMessage, peer_id: NodeId) {
        let txid = *msg.inner();
        let we_know_tx = self.knows_transactions(&txid);
        log::info!("Received {msg} from peer {peer_id}");
        if let Some(peer) = self.get_peer_mut(peer_id) {
            if msg.is_inv() {
                peer.add_known_transaction(txid);
                if !we_know_tx {
                    log::info!("Transaction unknown. Sending getdata to peer {peer_id}");
                    peer.enqueue_message(NetworkMessage::GETDATA(txid));
                }
            } else if msg.is_get_data() {
                assert!(!peer.knows_transactions(&txid), "Received a transaction request (txid: {txid}) from a node that already knows about it (peer_id {peer_id})");
                log::info!("Sending tx(txid: {txid}) to peer {peer_id}");
                peer.enqueue_message(NetworkMessage::TX(txid));
            } else {
                assert!(peer.knows_transactions(&txid), "Received a transaction (txid: {txid}) from a node that shouldn't know about it (peer_id {peer_id})");
                self.add_known_transaction(txid);
            }
        }
    }
}
