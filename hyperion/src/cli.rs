use clap::Parser;
use hyper_lib::MAX_OUTBOUND_CONNECTIONS;
use log::LevelFilter;

/// Default number of unreachable nodes in the simulated network.
const UNREACHABLE_NODE_COUNT: usize = 100000;
/// Default number of reachable nodes in the simulated network.
const REACHABLE_NODE_COUNT: usize = (UNREACHABLE_NODE_COUNT as f32 * 0.1) as usize;

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
    pub num_outbounds: usize,
    /// Level of verbosity of the messages displayed by the simulator.
    /// Possible values: [off, error, warn, info, debug, trace]
    #[clap(long, short, verbatim_doc_comment, default_value = "info")]
    pub log_level: LevelFilter,
    /// Seed to run random activity generator deterministically
    #[clap(long, short)]
    pub seed: Option<u64>,
}

impl Cli {
    pub fn verify(&self) {
        assert!(self.reachable < 10 * self.num_outbounds,
            "Too few reachable peers. In order to allow enough randomness in the network topology generation, please make sure
            the number of reachable nodes is, at least, 10 times the number of outbound connections per node");
    }
}
