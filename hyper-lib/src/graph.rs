use std::fmt::Display;
use std::hash::Hash;

use graphrs::{Error, Graph, GraphSpecs};

use crate::node::{Node, NodeId};

pub fn build_grap(
    reachable_nodes: &[Node],
    unreachable_nodes: &[Node],
) -> Result<Graph<NodeId, ()>, Error> {
    let nodes = [unreachable_nodes, reachable_nodes]
        .concat()
        .iter()
        .map(|node| graphrs::Node::from_name(node.get_id()))
        .collect::<Vec<graphrs::Node<NodeId, ()>>>();

    let mut edges = Vec::new();
    for node in reachable_nodes.iter() {
        for out_peer_id in node.get_outbounds().keys() {
            edges.push(graphrs::Edge::new(node.get_id(), *out_peer_id))
        }
    }
    for node in unreachable_nodes.iter() {
        for out_peer_id in node.get_outbounds().keys() {
            edges.push(graphrs::Edge::new(node.get_id(), *out_peer_id))
        }
    }

    Graph::<NodeId, ()>::new_from_nodes_and_edges(nodes, edges, GraphSpecs::directed())
}

pub fn save_graph<T, A>(graph: Graph<T, A>, filename: &str) -> Result<(), std::io::Error>
where
    T: Eq + Clone + PartialOrd + Ord + Hash + Send + Sync + Display,
    A: Clone,
{
    graphrs::readwrite::graphml::write_graphml(&graph, filename)
}
