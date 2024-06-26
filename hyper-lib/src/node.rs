use std::collections::HashSet;

pub type NodeId = u32;

#[derive(Clone)]
pub struct Node {
    node_id: NodeId,
    is_reachable: bool,
    in_peers: HashSet<NodeId>,
    out_peers: HashSet<NodeId>,
}

impl Node {
    pub fn new(node_id: NodeId, is_reachable: bool) -> Self {
        Node {
            node_id,
            is_reachable,
            in_peers: HashSet::new(),
            out_peers: HashSet::new(),
        }
    }

    pub fn get_id(&self) -> NodeId {
        self.node_id
    }

    pub fn get_inbounds(&self) -> &HashSet<NodeId> {
        &self.in_peers
    }

    pub fn get_outbounds(&self) -> &HashSet<NodeId> {
        &self.out_peers
    }

    pub fn connect(&mut self, peer_id: NodeId) {
        assert!(
            !self.in_peers.contains(&peer_id),
            "Peer {peer_id} is already connected to us"
        );
        assert!(
            self.out_peers.insert(peer_id),
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
            !self.out_peers.contains(&peer_id),
            "We (node_id: {}) are already connected to peer {peer_id}",
            self.node_id
        );
        assert!(
            self.in_peers.insert(peer_id),
            "Peer {peer_id} is already connected to us"
        );
    }
}
