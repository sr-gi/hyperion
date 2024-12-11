use std::{path::PathBuf, str::FromStr};

use clap::Parser;
use indicatif::{ProgressIterator, ProgressStyle};
use log::LevelFilter;
use simple_logger::SimpleLogger;

use hyper_lib::simulator::{Event, Simulator};
use hyperion::cli::Cli;

fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();
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
        &mut cli.seed,
        !cli.no_latency,
    );

    // Pick a (source) node to broadcast the target transaction from.
    let source_node_id = simulator.get_random_nodeid();
    let reachable_source = source_node_id < cli.reachable;
    log::info!(
        "Starting simulation: broadcasting transaction from node {source_node_id} ({})",
        if reachable_source {
            "reachable"
        } else {
            "unreachable"
        }
    );
    if cli.n > 1 {
        log::info!(
            "The simulation will be run {} times and results will be averaged",
            cli.n
        );
    }

    let start_time = 0;
    let mut overall_time = 0;

    // Display a progress bar only if we are running in multi-simulation mode
    let sty = ProgressStyle::with_template(if cli.n > 1 {
        "Simulating [{wide_bar:.cyan/blue} {pos:>2}/{len:2}] {elapsed_precise}"
    } else {
        ""
    })
    .unwrap()
    .progress_chars("##-");

    for _ in (0..cli.n).progress().with_style(sty) {
        // For statistical purposes
        let mut nodes_reached = 1;
        let mut percentile_time = 0;

        // Bootstrap the set reconciliation events (if needed) and send out the transaction.
        // All simulations start at time 0 so we don't have to carry any offset when computing
        // the overall time
        simulator.schedule_set_reconciliation(start_time);
        for e in simulator
            .get_node_mut(source_node_id)
            .unwrap()
            .broadcast_tx(start_time)
        {
            simulator.add_event(e);
        }
        // Record the initial time as the time when the first node sends out the transaction.
        // We don't need to account for the time the source withholds it
        let first_seen_time = simulator.get_next_event_time().unwrap();

        // Process events until the queue is empty
        while let Some(scheduled_event) = simulator.get_next_event() {
            let (event, current_time) = scheduled_event.into();
            match event {
                Event::ReceiveMessageFrom(src, dst, msg) => {
                    if msg.is_tx() && percentile_time == 0 {
                        nodes_reached += 1;
                        if nodes_reached as f32 >= target_node_count {
                            percentile_time = current_time - first_seen_time;
                        }
                    }
                    for future_event in simulator
                        .network
                        .get_node_mut(dst)
                        .unwrap()
                        .receive_message_from(msg, src, current_time)
                    {
                        simulator.add_event(future_event);
                    }
                }
                Event::ProcessScheduledAnnouncement(src, dst) => {
                    if let Some(scheduled_event) = simulator
                        .network
                        .get_node_mut(src)
                        .unwrap()
                        .process_scheduled_announcement(dst, current_time)
                    {
                        simulator.add_event(scheduled_event);
                    }
                }
                Event::ProcessDelayedRequest(target, peer_id) => {
                    if let Some(delayed_event) = simulator
                        .network
                        .get_node_mut(target)
                        .unwrap()
                        .process_delayed_request(peer_id, current_time)
                    {
                        simulator.add_event(delayed_event);
                    }
                }
                Event::ProcessScheduledReconciliation(src) => {
                    // Drop the periodic schedule if there is nothing else to be reconciled
                    // This allow us to finish the simulation, for Erlay scenarios, by consuming
                    // all messages in the queue
                    let node = simulator.network.get_node(src).unwrap();
                    if !node.knows_transaction()
                        || !node.get_outbounds().keys().all(|node_id| {
                            simulator
                                .network
                                .get_node(*node_id)
                                .unwrap()
                                .knows_transaction()
                        })
                    {
                        // Processing an scheduled reconciliation will return the reconciliation flow
                        // start, plus the scheduling of the next reconciliation (with the next peer in line)
                        let (rec_req, next_event) = simulator
                            .network
                            .get_node_mut(src)
                            .unwrap()
                            .process_scheduled_reconciliation(current_time);
                        simulator.add_event(rec_req);
                        simulator.add_event(next_event);
                    }
                }
            }
        }

        // Make sure every node has received the transaction
        for node in simulator.network.get_nodes() {
            assert!(node.knows_transaction());
        }

        assert_ne!(percentile_time, 0);
        overall_time += percentile_time;

        for node in simulator.network.get_nodes_mut() {
            node.reset();
        }
    }

    let avg_percentile_time = (overall_time as f32 / cli.n as f32).round() as u64;
    let statistics = simulator.network.get_statistics();
    let output_result = hyper_lib::OutputResult::new(
        cli.percentile_target,
        avg_percentile_time,
        statistics,
        cli.get_simulation_params(),
        cli.seed.unwrap(),
    );
    output_result.display();

    // Store data in csv to the specified output file (if any)
    if let Some(of) = cli.output_file {
        let mut output_file = PathBuf::from_str(&of)?;
        if let Some(a) = of.strip_prefix('~') {
            if let Some(b) = of.strip_prefix("~/") {
                output_file = home::home_dir().unwrap().join(b)
            } else {
                output_file = home::home_dir().unwrap().join(a)
            }
        };

        log::info!("Storing results in {}", output_file.to_str().unwrap());
        let mut wtr = csv::WriterBuilder::new()
            // Only add headers if the file doesn't exist. Assume file is properly formatted otherwise
            .has_headers(!std::fs::exists(&output_file)?)
            .from_writer(
                std::fs::OpenOptions::new()
                    // create if missing + append
                    .create(true)
                    .append(true)
                    .open(&output_file)?,
            );
        wtr.serialize(output_result)?;
        wtr.flush()?;
    }

    Ok(())
}
