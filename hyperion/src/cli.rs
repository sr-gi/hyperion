use clap::Parser;
use log::LevelFilter;

use hyper_lib::{SimulationParameters, MAX_OUTBOUND_CONNECTIONS};

/// Default number of unreachable nodes in the simulated network.
const UNREACHABLE_NODE_COUNT: usize = 100000;
/// Default number of reachable nodes in the simulated network.
const REACHABLE_NODE_COUNT: usize = (UNREACHABLE_NODE_COUNT as f32 * 0.1) as usize;
/// Target for the transaction propagation statistics snapshot
const TARGET_PERCENTILE: u16 = 90;

#[derive(Parser)]
#[command(version, about)]
pub struct Cli {
    /// The number of reachable nodes in the simulated network.
    #[clap(long, short, default_value_t = REACHABLE_NODE_COUNT)]
    pub reachable: usize,
    /// The number of unreachable nodes in the simulated network.
    #[clap(long, short, default_value_t = UNREACHABLE_NODE_COUNT, requires="reachable")]
    pub unreachable: usize,
    /// The number of outbound connections established per node.
    #[clap(long, short, default_value_t = MAX_OUTBOUND_CONNECTIONS)]
    pub outbounds: usize,
    /// Level of verbosity of the messages displayed by the simulator.
    /// Possible values: [off, error, warn, info, debug, trace]
    #[clap(long, short, verbatim_doc_comment, default_value = "info")]
    pub log_level: LevelFilter,
    /// Propagation percentile target. Use to measure transaction propagation times
    #[clap(long, short, default_value_t = TARGET_PERCENTILE, value_parser = clap::value_parser!(u16).range(1..101))]
    pub percentile_target: u16,
    /// Whether or not nodes in the simulation support Erlay (all of them for now, this is likely to change)
    #[clap(long)]
    pub erlay: bool,
    /// Seed to run random activity generator deterministically
    #[clap(long, short)]
    pub seed: Option<u64>,
    /// Don't add network latency to messages exchanges between peers. Useful for debugging
    #[clap(long)]
    pub no_latency: bool,
    /// Number of times the simulation will be repeated. Smooths the statistical results
    #[clap(short, default_value_t = 1)]
    pub n: u32,
    /// An output file name where to store simulation results to, in csv.
    /// If the file already exists, data will be appended to it, otherwise it will be created
    #[clap(long, visible_alias = "out", verbatim_doc_comment)]
    pub output_file: Option<String>,
}

impl Cli {
    pub fn verify(&self) {
        assert!(self.reachable >= 10 * self.outbounds,
            "Too few reachable peers. In order to allow enough randomness in the network topology generation, please make sure
            the number of reachable nodes is, at least, 10 times the number of outbound connections per node");
    }

    pub fn get_simulation_params(&self) -> SimulationParameters {
        SimulationParameters::new(self.n, self.reachable, self.unreachable, self.erlay)
    }
}
