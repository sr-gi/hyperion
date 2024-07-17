use std::iter::Sum;

use crate::network::NetworkMessage;

/// Statistics about how many messages of a certain type has a node sent/received
#[derive(Clone)]
pub struct NodeStatistics {
    inv: (u32, u32),
    get_data: (u32, u32),
    tx: (u32, u32),
    bytes_sent: u32,
    bytes_received: u32,
}

impl NodeStatistics {
    pub fn new() -> Self {
        NodeStatistics {
            inv: (0, 0),
            get_data: (0, 0),
            tx: (0, 0),
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    /// Adds a sent message to the statistics
    pub fn add_sent(&mut self, msg: NetworkMessage) {
        match msg {
            NetworkMessage::INV(_) => self.inv.0 += 1,
            NetworkMessage::GETDATA(_) => self.get_data.0 += 1,
            NetworkMessage::TX(_) => self.tx.0 += 1,
        }
        self.bytes_sent += msg.get_size();
    }

    /// Adds a receive message to the statistics
    pub fn add_received(&mut self, msg: NetworkMessage) {
        match msg {
            NetworkMessage::INV(_) => self.inv.1 += 1,
            NetworkMessage::GETDATA(_) => self.get_data.1 += 1,
            NetworkMessage::TX(_) => self.tx.1 += 1,
        }
        self.bytes_received += msg.get_size();
    }

    pub fn get_sent_count(&self) -> u32 {
        self.inv.0 + self.get_data.0 + self.tx.0
    }

    pub fn get_received_count(&self) -> u32 {
        self.inv.1 + self.get_data.1 + self.tx.1
    }

    pub fn get_sent_bytes(&self) -> u32 {
        self.bytes_sent
    }

    pub fn get_received_bytes(&self) -> u32 {
        self.bytes_received
    }
}

impl Default for NodeStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for NodeStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "INVS: {}/{}. GETDATA: {}/{}. TX: {}/{}",
            self.inv.0, self.inv.1, self.get_data.0, self.get_data.1, self.tx.0, self.tx.1,
        )
    }
}

impl Sum for NodeStatistics {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(
            NodeStatistics {
                inv: (0, 0),
                get_data: (0, 0),
                tx: (0, 0),
                bytes_sent: 0,
                bytes_received: 0,
            },
            |a, b| NodeStatistics {
                inv: (a.inv.0 + b.inv.0, a.inv.1 + b.inv.1),
                get_data: (a.get_data.0 + b.get_data.0, a.get_data.1 + b.get_data.1),
                tx: (a.tx.0 + b.tx.0, a.tx.1 + b.tx.1),
                bytes_sent: a.bytes_sent + b.bytes_sent,
                bytes_received: a.bytes_received + b.bytes_received,
            },
        )
    }
}

pub type AccumulatedStatistics = NodeStatistics;

pub struct AveragedStatistics {
    avg_sent_reachable: f32,
    avg_received_reachable: f32,
    avg_sent_unreachable: f32,
    avg_received_unreachable: f32,
}

impl AveragedStatistics {
    pub fn sent_reachable(&self) -> f32 {
        self.avg_sent_reachable
    }

    pub fn received_reachable(&self) -> f32 {
        self.avg_received_reachable
    }

    pub fn sent_unreachable(&self) -> f32 {
        self.avg_sent_unreachable
    }

    pub fn received_unreachable(&self) -> f32 {
        self.avg_received_unreachable
    }
}

pub struct NetworkStatistics {
    reachable_stats: AccumulatedStatistics,
    reachable_count: usize,
    unreachable_stats: AccumulatedStatistics,
    unreachable_count: usize,
}

impl NetworkStatistics {
    pub fn new(
        reachable_stats: AccumulatedStatistics,
        reachable_count: usize,
        unreachable_stats: AccumulatedStatistics,
        unreachable_count: usize,
    ) -> Self {
        Self {
            reachable_stats,
            reachable_count,
            unreachable_stats,
            unreachable_count,
        }
    }

    pub fn avg_messages(&self) -> AveragedStatistics {
        AveragedStatistics {
            avg_sent_reachable: self.reachable_stats.get_sent_count() as f32
                / self.reachable_count as f32,
            avg_received_reachable: self.reachable_stats.get_received_count() as f32
                / self.reachable_count as f32,
            avg_sent_unreachable: self.unreachable_stats.get_sent_count() as f32
                / self.unreachable_count as f32,
            avg_received_unreachable: self.unreachable_stats.get_received_count() as f32
                / self.unreachable_count as f32,
        }
    }

    pub fn avg_bytes(&self) -> AveragedStatistics {
        AveragedStatistics {
            avg_sent_reachable: self.reachable_stats.get_sent_bytes() as f32
                / self.reachable_count as f32,
            avg_received_reachable: self.reachable_stats.get_received_bytes() as f32
                / self.reachable_count as f32,
            avg_sent_unreachable: self.unreachable_stats.get_sent_bytes() as f32
                / self.unreachable_count as f32,
            avg_received_unreachable: self.unreachable_stats.get_received_bytes() as f32
                / self.unreachable_count as f32,
        }
    }
}
