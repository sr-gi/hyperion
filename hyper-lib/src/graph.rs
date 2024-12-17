use std::fmt::Display;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use hashbrown::HashMap;
use rand::rngs::StdRng;

use crate::network::{Link, Network};
use crate::node::{Node, NodeId};
use crate::parse_graph::{Graph, NodeData};

#[derive(Clone)]
pub struct NodeAttributes {
    is_reachable: bool,
    is_erlay: bool,
}

impl TryFrom<&Network> for Graph {
    type Error = graphrs::Error;

    fn try_from(network: &Network) -> Result<Self, Self::Error> {
        let mut edges = Vec::new();
        for node in network.get_nodes() {
            for out_peer_id in node.get_outbounds().keys() {
                let link: Link = (node.get_id(), *out_peer_id).into();
                edges.push((
                    node.get_id().to_string(),
                    out_peer_id.to_string(),
                    *network.get_links().get(&link).unwrap(),
                ))
            }
        }

        let mut nodes = network
            .get_nodes()
            .iter()
            .map(|node| {
                NodeData::new(
                    node.get_id().to_string(),
                    HashMap::from([
                        ("is_reachable".to_string(), node.is_reachable().to_string()),
                        ("is_erlay".to_string(), node.is_erlay().to_string()),
                    ]),
                )
            })
            .collect::<Vec<_>>();

        Ok(Graph::new(nodes, edges))
    }
}

impl From<(Graph, Arc<Mutex<StdRng>>)> for Network {
    fn from(value: (Graph, Arc<Mutex<StdRng>>)) -> Self {
        let graph = value.0;
        let rng = value.1;

        let mut nodes = graph
            .get_nodes()
            .into_iter()
            .map(|n| {
                Node::new(
                    n.get_id(),
                    rng.clone(),
                    n.get_attributes().get("is_reachable").unwrap(),
                    n.get_attributes().get("is_erlay").unwrap(),
                )
            })
            .collect::<Vec<_>>();

        // Sort the vector in ascending order so we can get nodes by position using their node_id
        nodes.sort_by_key(|a| a.get_id());

        // Create all connections
        let mut links = HashMap::new();
        for (src_id, dst_id, latency) in graph.get_edges().iter() {
            {
                let src = nodes.get_mut(src_id).unwrap();
                src.connect(dst_id, src.is_erlay());
            }
            {
                let dst = nodes.get_mut(dst_id).unwrap();
                dst.accept_connection(src_id, dst.is_erlay());
            }
            links.insert((src_id, dst_id).into(), weight as u64);
        }

        Network::from_connected_nodes_and_links(nodes, links)
    }
}

// pub fn save_graph<T, A>(graph: &Graph<T, A>, filename: &str) -> Result<(), std::io::Error>
// where
//     T: Eq + Clone + PartialOrd + Ord + Hash + Send + Sync + Display,
//     A: Clone,
// {
//     graphrs::readwrite::graphml::write_graphml(graph, filename)
// }

// WIP: Turns out you cannot load into Graph<T, A> because the attributes are actually never backed up SMH
// Find another graph lib...
// pub fn load_graph(filename: &str) -> Result<Graph<String, ()>, graphrs::Error> {
//     let g = graphrs::readwrite::graphml::read_graphml(filename, GraphSpecs::directed());
//     log::info!("{:?}", g.unwrap().get_all_nodes());
//     graphrs::readwrite::graphml::read_graphml(filename, GraphSpecs::directed())
// }
