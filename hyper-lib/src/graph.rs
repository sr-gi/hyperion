use std::fmt::Display;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use graphrs::{Graph, GraphSpecs};
use hashbrown::HashMap;
use rand::rngs::StdRng;

use crate::network::{Link, Network};
use crate::node::{Node, NodeId};

#[derive(Clone)]
pub struct NodeAttributes {
    is_reachable: bool,
    is_erlay: bool,
}

impl TryFrom<&Network> for Graph<NodeId, NodeAttributes> {
    type Error = graphrs::Error;

    fn try_from(network: &Network) -> Result<Self, Self::Error> {
        let mut edges = Vec::new();
        for node in network.get_nodes() {
            for out_peer_id in node.get_outbounds().keys() {
                let link: Link = (node.get_id(), *out_peer_id).into();
                edges.push(graphrs::Edge::with_weight(
                    node.get_id(),
                    *out_peer_id,
                    *network.get_links().get(&link).unwrap() as f64,
                ))
            }
        }

        Graph::<NodeId, NodeAttributes>::new_from_nodes_and_edges(
            network
                .get_nodes()
                .iter()
                .map(|node| {
                    graphrs::Node::from_name_and_attributes(
                        node.get_id(),
                        NodeAttributes {
                            is_reachable: node.is_reachable(),
                            is_erlay: node.is_erlay(),
                        },
                    )
                })
                .collect::<Vec<graphrs::Node<NodeId, NodeAttributes>>>(),
            edges,
            GraphSpecs::directed(),
        )
    }
}

impl From<(Graph<NodeId, NodeAttributes>, Arc<Mutex<StdRng>>)> for Network {
    fn from(value: (Graph<NodeId, NodeAttributes>, Arc<Mutex<StdRng>>)) -> Self {
        let graph = value.0;
        let rng = value.1;

        let mut nodes = graph
            .get_all_nodes()
            .into_iter()
            .map(|n| {
                Node::new(
                    n.name,
                    rng.clone(),
                    n.attributes.as_ref().unwrap().is_reachable,
                    n.attributes.as_ref().unwrap().is_erlay,
                )
            })
            .collect::<Vec<_>>();

        // Sort the vector in ascending order so we can get nodes by position using their node_id
        nodes.sort_by_key(|a| a.get_id());

        // Create all connections
        let mut links = HashMap::new();
        for edge in graph.get_all_edges().iter() {
            {
                let src = nodes.get_mut(edge.u).unwrap();
                src.connect(edge.v, src.is_erlay());
            }
            {
                let dst = nodes.get_mut(edge.v).unwrap();
                dst.accept_connection(edge.u, dst.is_erlay());
            }
            links.insert((edge.u, edge.v).into(), edge.weight as u64);
        }

        Network::from_connected_nodes_and_links(nodes, links)
    }
}

pub fn save_graph<T, A>(graph: &Graph<T, A>, filename: &str) -> Result<(), std::io::Error>
where
    T: Eq + Clone + PartialOrd + Ord + Hash + Send + Sync + Display,
    A: Clone,
{
    graphrs::readwrite::graphml::write_graphml(graph, filename)
}

// WIP: Turns out you cannot load into Graph<T, A> because the attributes are actually never backed up SMH
// Find another graph lib...
// pub fn load_graph(filename: &str) -> Result<Graph<String, ()>, graphrs::Error> {
//     let g = graphrs::readwrite::graphml::read_graphml(filename, GraphSpecs::directed());
//     log::info!("{:?}", g.unwrap().get_all_nodes());
//     graphrs::readwrite::graphml::read_graphml(filename, GraphSpecs::directed())
// }
