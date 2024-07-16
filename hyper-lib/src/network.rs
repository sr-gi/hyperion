use crate::node::{Node, NodeId};
use crate::{TxId, MAX_OUTBOUND_CONNECTIONS};

use std::collections::HashSet;

use rand::distributions::{Distribution, Uniform};
use rand::rngs::StdRng;

/// Defines the collection of network messages that can be exchanged between peers
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub enum NetworkMessage {
    INV(TxId),
    GETDATA(TxId),
    TX(TxId),
}

impl NetworkMessage {
    pub fn inner(&self) -> &TxId {
        match self {
            NetworkMessage::INV(x) => x,
            NetworkMessage::GETDATA(x) => x,
            NetworkMessage::TX(x) => x,
        }
    }

    /// Returns the size of the given network message
    fn get_size(&self) -> u32 {
        // FIXME: Add some sizes that make sense
        match self {
            NetworkMessage::INV(_) => 32,
            NetworkMessage::GETDATA(_) => 32,
            NetworkMessage::TX(_) => 150,
        }
    }

    /// Returns whether the network message is a inventory message
    pub fn is_inv(&self) -> bool {
        matches!(self, NetworkMessage::INV(..))
    }

    /// Returns whether the network message is a data request message
    pub fn is_get_data(&self) -> bool {
        matches!(self, NetworkMessage::GETDATA(..))
    }

    /// Returns whether the network message is transaction message
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
    /// Collection of nodes that composes the simulated network
    nodes: Vec<Node>,
}

impl Network {
    pub fn new(reachable_count: usize, unreachable_count: usize, rng: &mut StdRng) -> Self {
        let mut reachable_nodes = (0..reachable_count)
            .map(|i| Node::new(i, rng.clone(), true))
            .collect::<Vec<_>>();
        let mut unreachable_nodes: Vec<Node> = (reachable_count
            ..reachable_count + unreachable_count)
            .map(|i| Node::new(i, rng.clone(), false))
            .collect::<Vec<_>>();

        log::info!(
            "Creating nodes ({}: {} reachable, {} unreachable)",
            reachable_count + unreachable_count,
            reachable_count,
            unreachable_count
        );

        let peers_die = Uniform::from(0..reachable_nodes.len());

        log::info!(
            "Connecting unreachable nodes to reachable ({} connections)",
            unreachable_count * MAX_OUTBOUND_CONNECTIONS
        );
        Network::connect_unreachable(
            &mut unreachable_nodes,
            &mut reachable_nodes,
            rng,
            &peers_die,
        );

        log::info!(
            "Connecting reachable nodes to reachable ({} connections)",
            reachable_count * MAX_OUTBOUND_CONNECTIONS
        );
        Network::connect_reachable(&mut reachable_nodes, rng, &peers_die);

        let mut nodes = reachable_nodes;
        nodes.extend(unreachable_nodes);

        Self { nodes }
    }

    /// Connects a collection of unreachable nodes to a collection of reachable ones.
    /// A given pair of nodes will have, at most, one connection between them.
    /// Nodes to be connected to are picked at random given an uniform distribution [dist]
    fn connect_unreachable(
        unreachable_nodes: &mut [Node],
        reachable_nodes: &mut [Node],
        rng: &mut StdRng,
        dist: &Uniform<NodeId>,
    ) {
        for node in unreachable_nodes.iter_mut() {
            let mut already_connected_to = HashSet::new();
            for _ in 0..MAX_OUTBOUND_CONNECTIONS {
                let peer_id = Network::get_peer_to_connect(&mut already_connected_to, rng, dist);
                node.connect(peer_id);
                reachable_nodes
                    .get_mut(peer_id)
                    .unwrap_or_else(|| {
                        panic!("Cannot connect to reachable peer {peer_id}. Peer not found")
                    })
                    .accept_connection(node.get_id());
            }
        }
    }

    /// Connects a collection of reachable nodes between them.
    /// A given pair of nodes will have, at most, one connection between them.
    /// Nodes to be connected to are picked at random given an uniform distribution [dist]
    fn connect_reachable(reachable_nodes: &mut [Node], rng: &mut StdRng, dist: &Uniform<NodeId>) {
        for node_id in 0..reachable_nodes.len() {
            let mut already_connected_to = reachable_nodes[node_id]
                .get_inbounds()
                .keys()
                .cloned()
                .collect::<HashSet<_>>();
            already_connected_to.insert(node_id);

            for _ in 0..MAX_OUTBOUND_CONNECTIONS {
                let peer_id = Network::get_peer_to_connect(&mut already_connected_to, rng, dist);

                let (node, peer) = if peer_id < node_id {
                    let (r1, r2) = reachable_nodes.split_at_mut(node_id);
                    let peer = &mut r1[peer_id];
                    let node = &mut r2[0];
                    (node, peer)
                } else {
                    let (r1, r2) = reachable_nodes.split_at_mut(peer_id);
                    let node = &mut r1[node_id];
                    let peer = &mut r2[0];
                    (node, peer)
                };

                node.connect(peer_id);
                peer.accept_connection(node_id);
            }
        }
    }

    /// Utility function get a node we can connect to, given a list of nodes we are already connected to
    /// and an uniform distribution [dist]
    fn get_peer_to_connect(
        already_connected_to: &mut HashSet<NodeId>,
        rng: &mut StdRng,
        dist: &Uniform<usize>,
    ) -> NodeId {
        let mut peer_id = dist.sample(rng);
        while already_connected_to.contains(&peer_id) {
            peer_id = dist.sample(rng);
        }
        already_connected_to.insert(peer_id);

        peer_id
    }

    pub fn get_node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn get_node(&self, node_id: NodeId) -> Option<&Node> {
        self.nodes.get(node_id)
    }

    pub fn get_node_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(node_id)
    }

    pub fn get_nodes(&self) -> &Vec<Node> {
        &self.nodes
    }
}
