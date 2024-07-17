pub mod graph;
pub mod network;
pub mod node;
pub mod simulator;
pub mod statistics;

pub type TxId = u32;

pub const MAX_OUTBOUND_CONNECTIONS: usize = 8;
static SECS_TO_NANOS: u64 = 1_000_000_000;
