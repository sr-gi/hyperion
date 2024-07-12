use log::LevelFilter;
use simple_logger::SimpleLogger;

use hyper_lib::simulator::{Event, Simulator};

// TODO: We need to do some basic assertions here to make sure the proposed network is feasible to build.
// A network where REACHABLE_NODE_COUNT is a big proportion (or even bigger) than MAX_OUTBOUND_CONNECTIONS
// would lead to reachable nodes not being able to achieve MAX_OUTBOUND_CONNECTIONS (since there won't be
// enough distinct nodes to pick from). In the current state, the code will try to build the network forever.
const UNREACHABLE_NODE_COUNT: usize = 100000;
const REACHABLE_NODE_COUNT: usize = (UNREACHABLE_NODE_COUNT as f32 * 0.1) as usize;

fn main() -> anyhow::Result<()> {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .with_module_level("hyper_lib", LevelFilter::Info)
        .with_module_level("hyperion", LevelFilter::Info)
        .init()
        .unwrap();

    // This is just a wrapper so we can end up providing our own seed if needed
    let mut simulator = Simulator::new(REACHABLE_NODE_COUNT, UNREACHABLE_NODE_COUNT);

    // Pick a (source) node to broadcast the target transaction from.
    let txid = simulator.get_random_txid();
    let source_node_id = simulator.get_random_nodeid();
    let source_node = simulator.get_node_mut(source_node_id).unwrap();

    log::info!(
        "Starting simulation: broadcasting transaction (txid: {txid:x}) from node {source_node_id}"
    );
    for (event, time) in source_node.broadcast_tx(txid, 0) {
        simulator.add_event(event, time);
    }

    // Process events until the queue is empty
    while let Some((event, time)) = simulator.get_next_event() {
        match event {
            Event::SampleNewInterval(target, peer_id) => {
                simulator
                    .network
                    .get_node_mut(target)
                    .unwrap()
                    .get_next_announcement_time(time, peer_id);
            }
            Event::ReceiveMessageFrom(src, dst, msg) => {
                for (future_event, future_time) in simulator
                    .network
                    .get_node_mut(dst)
                    .unwrap()
                    .receive_message_from(msg, src, time)
                {
                    simulator.add_event(future_event, future_time);
                }
            }
            Event::ProcessScheduledAnnouncement(src, dst, txid) => {
                if let Some((scheduled_event, t)) = simulator
                    .network
                    .get_node_mut(src)
                    .unwrap()
                    .process_scheduled_announcement(dst, txid, time)
                {
                    simulator.add_event(scheduled_event, t);
                }
            }
            Event::ProcessDelayedRequest(target, txid) => {
                if let Some((delayed_event, next_interval)) = simulator
                    .network
                    .get_node_mut(target)
                    .unwrap()
                    .process_delayed_request(txid, time)
                {
                    simulator.add_event(delayed_event, next_interval);
                }
            }
        }
    }

    // Make sure every node has received the transaction
    for node in simulator.network.get_nodes() {
        assert!(node.knows_transaction(&txid));
        log::info!("Node {}: {}", node.get_id(), node.get_statistics());
    }

    log::info!("Transaction has reached all nodes");

    Ok(())
}
