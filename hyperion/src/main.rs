use clap::Parser;
use log::LevelFilter;
use simple_logger::SimpleLogger;
use std::time;

use hyper_lib::simulator::{Event, Simulator};
use hyperion::cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.verify();

    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .with_module_level("hyper_lib", cli.log_level)
        .with_module_level("hyperion", cli.log_level)
        .init()
        .unwrap();

    let node_count = cli.reachable + cli.unreachable;
    let target_node_count = node_count as f32 * (cli.percentile_target as f32 / 100.0);
    let mut simulator = Simulator::new(
        cli.reachable,
        cli.unreachable,
        cli.outbounds,
        cli.erlay,
        cli.seed,
        !cli.no_latency,
    );

    // Pick a (source) node to broadcast the target transaction from.
    let source_node_id = simulator.get_random_nodeid();
    log::info!("Starting simulation: broadcasting transaction from node {source_node_id}");
    if cli.n > 1 {
        log::info!(
            "The simulation will be run {} times and results will be averaged",
            cli.n
        );
    }

    let start_time = 0;
    let mut overall_time = 0;
    let mut txid = simulator.get_random_txid();
    let mut sent_txs = Vec::new();
    for _ in 0..cli.n {
        // For statistical purposes
        let mut nodes_reached = 1;
        let mut percentile_time = 0;

        // Bootstrap the set reconciliation events (if needed) and send out the transaction.
        // All simulations start at time 0 so we don't have to carry any offset when computing
        // the overall time
        simulator.schedule_set_reconciliation(start_time);
        for (event, time) in simulator
            .get_node_mut(source_node_id)
            .unwrap()
            .broadcast_tx(txid, start_time)
        {
            simulator.add_event(event, time);
        }

        // Process events until the queue is empty
        while let Some((event, event_time)) = simulator.get_next_event() {
            match event {
                Event::ReceiveMessageFrom(src, dst, msg) => {
                    if msg.is_tx() && percentile_time == 0 {
                        nodes_reached += 1;
                        if nodes_reached as f32 >= target_node_count {
                            percentile_time = event_time;
                        }
                    }
                    for (future_event, future_time) in simulator
                        .network
                        .get_node_mut(dst)
                        .unwrap()
                        .receive_message_from(msg, src, event_time)
                    {
                        simulator.add_event(future_event, future_time);
                    }
                }
                Event::ProcessScheduledAnnouncement(src, dst) => {
                    if let Some((scheduled_event, t)) = simulator
                        .network
                        .get_node_mut(src)
                        .unwrap()
                        .process_scheduled_announcement(dst, event_time)
                    {
                        simulator.add_event(scheduled_event, t);
                    }
                }
                Event::ProcessDelayedRequest(target, txid) => {
                    if let Some((delayed_event, next_interval)) = simulator
                        .network
                        .get_node_mut(target)
                        .unwrap()
                        .process_delayed_request(txid, event_time)
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
                            .process_scheduled_reconciliation(event_time);
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

        // Pick a new txid for the next iteration (if any)
        if cli.n > 1 {
            sent_txs.push(txid);
            let mut new_txid = simulator.get_random_txid();
            while sent_txs.contains(&new_txid) {
                new_txid = simulator.get_random_txid();
            }
            txid = new_txid;
        }

        assert_ne!(percentile_time, 0);
        overall_time += percentile_time;
    }

    let avg_percentile_time = (overall_time as f32 / cli.n as f32).round() as u64;
    log::info!(
        "Transaction reached {}% of nodes in the network in {}s",
        cli.percentile_target,
        time::Duration::from_nanos(avg_percentile_time).as_secs_f32(),
    );

    let statistics = simulator.network.get_statistics();
    log::info!(
        "Reachable nodes sent/received {}/{} messages ({}/{} bytes) (avg)",
        statistics.avg_messages().sent_reachable() / cli.n as f32,
        statistics.avg_messages().received_reachable() / cli.n as f32,
        statistics.avg_bytes().sent_reachable() / cli.n as f32,
        statistics.avg_bytes().received_reachable() / cli.n as f32,
    );
    log::info!(
        "Unreachable nodes sent/received {}/{} messages ({}/{} bytes) (avg)",
        statistics.avg_messages().sent_unreachable() / cli.n as f32,
        statistics.avg_messages().received_unreachable() / cli.n as f32,
        statistics.avg_bytes().sent_unreachable() / cli.n as f32,
        statistics.avg_bytes().received_unreachable() / cli.n as f32,
    );

    Ok(())
}
