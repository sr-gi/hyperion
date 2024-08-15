pub mod graph;
pub mod network;
pub mod node;
pub mod simulator;
pub mod statistics;
pub mod txreconciliation;

mod indexedmap;

pub type TxId = u32;

pub const MAX_OUTBOUND_CONNECTIONS: usize = 8;
static SECS_TO_NANOS: u64 = 1_000_000_000;

#[cfg(test)]
mod test {
    use super::*;
    use rand::{self, RngCore};

    pub(crate) fn get_random_txid() -> TxId {
        rand::thread_rng().next_u32()
    }
}
