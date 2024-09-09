use clap::Parser;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use std::time;

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

    let start_time = 0;
    let node_count = cli.reachable + cli.unreachable;
    let mut simulator = Simulator::new(
        cli.reachable,
        cli.unreachable,
        cli.erlay,
        start_time,
        cli.seed,
        !cli.no_latency,
    );

    // Pick a (source) node to broadcast the target transaction from.
    let txid = simulator.get_random_txid();
    let source_node_id = simulator.get_random_nodeid();
    let source_node = simulator.get_node_mut(source_node_id).unwrap();

    log::info!(
        "Starting simulation: broadcasting transaction (txid: {txid:x}) from node {source_node_id}"
    );
    for (event, time) in source_node.broadcast_tx(txid, start_time) {
        simulator.add_event(event, time);
    }

    // For statistical purposes
    let mut nodes_reached = 1;
    let mut percentile_time = 0;
    let target_node_count = node_count as f32 * (cli.percentile_target as f32 / 100.0);

    // Process events until the queue is empty
    while let Some((event, time)) = simulator.get_next_event() {
        match event {
            Event::ReceiveMessageFrom(src, dst, msg) => {
                if msg.is_tx() && percentile_time == 0 {
                    nodes_reached += 1;
                    if nodes_reached as f32 >= target_node_count {
                        percentile_time = time;
                    }
                }
                for (future_event, future_time) in simulator
                    .network
                    .get_node_mut(dst)
                    .unwrap()
                    .receive_message_from(msg, src, time)
                {
                    simulator.add_event(future_event, future_time);
                }
            }
            Event::ProcessScheduledAnnouncement(src, dst) => {
                if let Some((scheduled_event, t)) = simulator
                    .network
                    .get_node_mut(src)
                    .unwrap()
                    .process_scheduled_announcement(dst, time)
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
            Event::ProcessScheduledReconciliation(src) => {
                // Drop the periodic schedule if there is nothing else to be reconciled
                // This allow us to finish the simulation, for Erlay scenarios, by consuming
                // all messages in the queue
                let node = simulator.network.get_node(src).unwrap();
                if !node.knows_transaction(&txid)
                    || !node.get_outbounds().keys().all(|node_id| {
                        simulator
                            .network
                            .get_node(*node_id)
                            .unwrap()
                            .knows_transaction(&txid)
                    })
                {
                    // Processing an scheduled reconciliation will return the reconciliation flow
                    // start, plus the scheduling of the next reconciliation (with the next peer in line)
                    let ((rec_req, req_time), (next_event, future_time)) = simulator
                        .network
                        .get_node_mut(src)
                        .unwrap()
                        .process_scheduled_reconciliation(time);
                    simulator.add_event(rec_req, req_time);
                    simulator.add_event(next_event, future_time);
                }
            }
        }
    }

    // Make sure every node has received the transaction
    for node in simulator.network.get_nodes() {
        assert!(node.knows_transaction(&txid));
    }

    let statistics = simulator.network.get_statistics();
    log::info!(
        "Reachable nodes sent/received {}/{} messages ({}/{} bytes) (avg)",
        statistics.avg_messages().sent_reachable(),
        statistics.avg_messages().received_reachable(),
        statistics.avg_bytes().sent_reachable(),
        statistics.avg_bytes().received_reachable(),
    );
    log::info!(
        "Unreachable nodes sent/received {}/{} messages ({}/{} bytes) (avg)",
        statistics.avg_messages().sent_unreachable(),
        statistics.avg_messages().received_unreachable(),
        statistics.avg_bytes().sent_unreachable(),
        statistics.avg_bytes().received_unreachable(),
    );

    log::info!(
        "Transaction (txid: {txid:x}) reached {}% of nodes in the network in {}s",
        cli.percentile_target,
        time::Duration::from_nanos(percentile_time).as_secs_f32(),
    );

    Ok(())
}
