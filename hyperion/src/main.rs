use std::collections::{HashMap, HashSet};
use std::sync::mpsc::channel;

use log::LevelFilter;
use rand::distributions::{Distribution, Uniform};
use rand::rngs::StdRng;
use rand::{thread_rng, Rng, RngCore, SeedableRng};
use simple_logger::SimpleLogger;

use hyper_lib::network::Network;
use hyper_lib::node::{Node, NodeId};
use hyper_lib::MAX_OUTBOUND_CONNECTIONS;

// TODO: We need to do some basic assertions here to make sure the proposed network is feasible to build.
// A network where REACHABLE_NODE_COUNT is a big proportion (or even bigger) than MAX_OUTBOUND_CONNECTIONS
// would lead to reachable nodes not being able to achieve MAX_OUTBOUND_CONNECTIONS (since there won't be
// enough distinct nodes to pick from). In the current state, the code will try to build the network forever.
const UNREACHABLE_NODE_COUNT: u32 = 500;
const REACHABLE_NODE_COUNT: u32 = (UNREACHABLE_NODE_COUNT as f32 * 0.1) as u32;
const TOTAL_NODE_COUNT: u32 = UNREACHABLE_NODE_COUNT + REACHABLE_NODE_COUNT;

fn get_peer_to_connect(
    already_connected_to: &mut HashSet<u32>,
    rng: &mut StdRng,
    peers_die: &Uniform<u32>,
) -> NodeId {
    let mut peer_id = peers_die.sample(rng);
    while already_connected_to.contains(&peer_id) {
        peer_id = peers_die.sample(rng);
    }
    already_connected_to.insert(peer_id);

    peer_id
}

fn main() -> anyhow::Result<()> {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .with_module_level("hyper_lib", LevelFilter::Info)
        .with_module_level("hyperion", LevelFilter::Info)
        .init()
        .unwrap();

    // This is just a wrapper so we can end up providing our own seed if needed
    let mut rng = StdRng::seed_from_u64(thread_rng().next_u64());
    // Uniform distribution so we can get peer ids in the reachable range
    let peers_die = Uniform::from(UNREACHABLE_NODE_COUNT..TOTAL_NODE_COUNT);

    log::info!(
        "Creating nodes ({}: {} unreachable, {} reachable)",
        TOTAL_NODE_COUNT,
        UNREACHABLE_NODE_COUNT,
        REACHABLE_NODE_COUNT
    );

    let (sender, receiver) = channel();
    let mut network = Network::new(TOTAL_NODE_COUNT as usize, receiver);

    // Create nodes
    let mut unreachable_nodes: Vec<Node> = (0..UNREACHABLE_NODE_COUNT)
        .map(|i| Node::new(i, false, sender.clone()))
        .collect::<Vec<_>>();
    let mut reachable_nodes = (UNREACHABLE_NODE_COUNT..TOTAL_NODE_COUNT)
        .map(|i| Node::new(i, true, sender.clone()))
        .collect::<Vec<_>>();

    log::info!(
        "Connecting unreachable nodes to reachable ({} connections)",
        UNREACHABLE_NODE_COUNT * MAX_OUTBOUND_CONNECTIONS as u32
    );
    // Connect unreachable peers to reachable (and accept connections)
    for node in unreachable_nodes.iter_mut() {
        let mut already_connected_to = HashSet::new();
        for _ in 0..MAX_OUTBOUND_CONNECTIONS {
            let peer_id: u32 = get_peer_to_connect(&mut already_connected_to, &mut rng, &peers_die);
            node.connect(peer_id);
            reachable_nodes
                .get_mut((peer_id - UNREACHABLE_NODE_COUNT) as usize)
                .ok_or_else(|| {
                    anyhow::anyhow!("Cannot connect to reachable {peer_id}. Peer not found")
                })?
                .accept_connection(node.get_id());
        }
    }

    log::info!(
        "Connecting reachable nodes to reachable ({} connections)",
        REACHABLE_NODE_COUNT * MAX_OUTBOUND_CONNECTIONS as u32
    );
    // Connect reachable peers among themselves
    let mut accept_connection_from: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for node in reachable_nodes.iter_mut() {
        // Initialize the set with our own id to prevents loops and extend it with all
        // needs that have already initiated a connection with us
        let mut already_connected_to = HashSet::from([node.get_id()]);
        already_connected_to.extend(
            accept_connection_from
                .get(&node.get_id())
                .unwrap_or(&Vec::new())
                .iter(),
        );

        for _ in 0..MAX_OUTBOUND_CONNECTIONS {
            let peer_id = get_peer_to_connect(&mut already_connected_to, &mut rng, &peers_die);
            node.connect(peer_id);

            if accept_connection_from.contains_key(&peer_id) {
                accept_connection_from
                    .get_mut(&peer_id)
                    .unwrap()
                    .push(node.get_id());
            } else {
                accept_connection_from.insert(peer_id, [node.get_id()].into());
            }
        }
    }
    network.nodes.extend(unreachable_nodes);

    log::debug!("Accepting connections from reachable nodes to reachable");
    // Accept connections (to reachable peer) from reachable peers
    // We need to split this in two since we are already borrowing reachable_nodes mutably
    // when iteration over them to create the connection, so we cannot borrow twice within
    // the same context
    for node in reachable_nodes.iter_mut() {
        for peer_id in accept_connection_from.get(&node.get_id()).unwrap() {
            node.accept_connection(*peer_id);
        }
    }
    network.nodes.extend(reachable_nodes);

    // Start the INV dispatch threads for each node
    for node in network.nodes.iter() {
        node.dispatch_invs();
        node.handle_delayed_requests();
    }

    // Pick a (source) node to broadcast the target transaction from.
    let txid = rng.next_u32();
    let source_node_id = rng.gen_range(0..TOTAL_NODE_COUNT);
    let source_node = network.nodes.get_mut(source_node_id as usize).unwrap();
    source_node.broadcast_tx(txid);

    // Start the "network" loop, that is, the message passing between nodes
    loop {
        let (msg, src, dst) = network.messages.recv().unwrap();
        log::info!("Received {msg} from node {src} to node {dst}");
        network
            .nodes
            .get_mut(dst as usize)
            .unwrap()
            .receive_message_from(msg, src);
    }
}
