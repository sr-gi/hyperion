use clap::Parser;
use log::LevelFilter;
use simple_logger::SimpleLogger;

use hyper_lib::simulator::{Event, Simulator};
use hyperion::cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .with_module_level("hyper_lib", cli.log_level)
        .with_module_level("hyperion", cli.log_level)
        .init()
        .unwrap();

    let mut simulator = Simulator::new(cli.reachable, cli.unreachable, cli.seed);

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
