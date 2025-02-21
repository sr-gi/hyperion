use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use statistics::NetworkStatistics;

pub mod graph;
pub mod network;
pub mod node;
pub mod simulator;
pub mod statistics;
pub mod txreconciliation;

pub const MAX_OUTBOUND_CONNECTIONS: usize = 8;
static SECS_TO_NANOS: u64 = 1_000_000_000;

pub struct SimulationParameters {
    /// Number of times the simulation will be repeated. Smooths the statistical results
    pub n: u32,
    /// The number of reachable nodes in the simulated network
    pub reachable: usize,
    /// The number of unreachable nodes in the simulated network
    pub unreachable: usize,
    /// Whether or not nodes in the simulation support Erlay (all of them for now)
    pub erlay: bool,
}

impl SimulationParameters {
    pub fn new(n: u32, reachable: usize, unreachable: usize, erlay: bool) -> Self {
        SimulationParameters {
            n,
            reachable,
            unreachable,
            erlay,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct OutputResult {
    timestamp: u64,
    percentile_target: u16,
    percentile_time: u64,
    full_propagation_time: u64,
    #[serde(rename = "r-sent-msgs")]
    reachable_sent_msgs: f32,
    #[serde(rename = "r-received-msgs")]
    reachable_received_msgs: f32,
    #[serde(rename = "r-msgs-volume")]
    reachable_msg_volume: f32,
    #[serde(rename = "r-sent-bytes")]
    reachable_sent_bytes: f32,
    #[serde(rename = "r-received-bytes")]
    reachable_received_bytes: f32,
    #[serde(rename = "r-bytes-volume")]
    reachable_bytes_volume: f32,
    #[serde(rename = "u-sent-msgs")]
    unreachable_sent_msgs: f32,
    #[serde(rename = "u-received-msgs")]
    unreachable_received_msgs: f32,
    #[serde(rename = "u-msgs-volume")]
    unreachable_msg_volume: f32,
    #[serde(rename = "u-sent-bytes")]
    unreachable_sent_bytes: f32,
    #[serde(rename = "u-received-bytes")]
    unreachable_received_bytes: f32,
    #[serde(rename = "u-bytes-volume")]
    unreachable_bytes_volume: f32,
    #[serde(rename = "r")]
    reachable_count: usize,
    #[serde(rename = "u")]
    unreachable_count: usize,
    in_poisson_mean: u16,
    out_poisson_mean: u16,
    out_fanout: u64,
    in_fanout: f32,
    n: u32,
    erlay: bool,
    seed: u64,
}

impl OutputResult {
    pub fn new(
        percentile_target: u16,
        percentile_time: u64,
        full_propagation_time: u64,
        statistics: NetworkStatistics,
        sim_params: SimulationParameters,
        seed: u64,
    ) -> Self {
        let n_float = sim_params.n as f32;
        let avg_msgs = statistics.avg_messages();
        let avg_bytes = statistics.avg_bytes();
        OutputResult {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            percentile_time,
            percentile_target,
            full_propagation_time,
            reachable_sent_msgs: avg_msgs.sent_reachable() / n_float,
            reachable_received_msgs: avg_msgs.received_reachable() / n_float,
            reachable_msg_volume: (avg_msgs.sent_reachable() + avg_msgs.received_reachable())
                / n_float,
            reachable_sent_bytes: avg_bytes.sent_reachable() / n_float,
            reachable_received_bytes: avg_bytes.received_reachable() / n_float,
            reachable_bytes_volume: (avg_bytes.sent_reachable() + avg_bytes.received_reachable())
                / n_float,
            unreachable_sent_msgs: avg_msgs.sent_unreachable() / n_float,
            unreachable_received_msgs: avg_msgs.received_unreachable() / n_float,
            unreachable_msg_volume: (avg_msgs.sent_unreachable() + avg_msgs.received_unreachable())
                / n_float,
            unreachable_sent_bytes: avg_bytes.sent_unreachable() / n_float,
            unreachable_received_bytes: avg_bytes.received_unreachable() / n_float,
            unreachable_bytes_volume: (avg_bytes.sent_unreachable()
                + avg_bytes.received_unreachable())
                / n_float,
            out_fanout: *crate::node::OUTBOUND_FANOUT_DESTINATIONS,
            in_fanout: (*crate::node::INBOUND_FANOUT_DESTINATIONS_FRACTION) as f32,
            in_poisson_mean: *crate::node::INBOUND_INVENTORY_BROADCAST_INTERVAL as u16,
            out_poisson_mean: *crate::node::OUTBOUND_INVENTORY_BROADCAST_INTERVAL as u16,
            n: sim_params.n,
            reachable_count: sim_params.reachable,
            unreachable_count: sim_params.unreachable,
            erlay: sim_params.erlay,
            seed,
        }
    }

    pub fn display(&self) {
        log::info!(
            "Transaction reached {}% of nodes in the network in {}s",
            self.percentile_target,
            Duration::from_nanos(self.percentile_time).as_secs_f32()
        );
        if self.percentile_target < 100 {
            log::info!(
                "Transaction reached all nodes in {}s",
                Duration::from_nanos(self.full_propagation_time).as_secs_f32()
            );
        }
        log::info!(
            "Reachable nodes data volume: {} messages ({} bytes) (avg)",
            self.reachable_msg_volume,
            self.reachable_bytes_volume
        );
        log::info!(
            "Unreachable nodes data volume: {} messages ({} bytes) (avg)",
            self.unreachable_msg_volume,
            self.unreachable_bytes_volume
        );
    }
}
