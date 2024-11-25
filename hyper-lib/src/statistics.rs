use std::iter::Sum;

use crate::network::NetworkMessage;

#[derive(Clone, Copy)]
struct Data {
    from_inbounds: u32,
    from_outbounds: u32,
    to_inbounds: u32,
    to_outbounds: u32,
}

impl Data {
    fn new() -> Self {
        Self {
            from_inbounds: 0,
            from_outbounds: 0,
            to_inbounds: 0,
            to_outbounds: 0,
        }
    }
}

impl Default for Data {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Add for Data {
    type Output = Data;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            from_inbounds: self.from_inbounds + rhs.from_inbounds,
            from_outbounds: self.from_outbounds + rhs.from_outbounds,
            to_inbounds: self.to_inbounds + rhs.to_inbounds,
            to_outbounds: self.to_outbounds + rhs.to_outbounds,
        }
    }
}

/// Statistics about how many messages of a certain type has a node sent/received
#[derive(Clone, Copy)]
pub struct NodeStatistics {
    inv: Data,
    get_data: Data,
    tx: Data,
    reqrecon: Data,
    sketch: Data,
    reconcildiff: Data,
    bytes: Data,
}

impl NodeStatistics {
    pub fn new() -> Self {
        NodeStatistics {
            inv: Data::new(),
            get_data: Data::new(),
            tx: Data::new(),
            bytes: Data::new(),
            reqrecon: Data::new(),
            sketch: Data::new(),
            reconcildiff: Data::new(),
        }
    }

    fn get_data_ref(&mut self, msg: &NetworkMessage) -> (&mut Data, &mut Data) {
        let data_ref = match msg {
            NetworkMessage::INV => &mut self.inv,
            NetworkMessage::GETDATA => &mut self.get_data,
            NetworkMessage::TX => &mut self.tx,
            NetworkMessage::REQRECON(_) => &mut self.reqrecon,
            NetworkMessage::SKETCH(_) => &mut self.sketch,
            NetworkMessage::RECONCILDIFF(_) => &mut self.reconcildiff,
        };

        (data_ref, &mut self.bytes)
    }

    /// Adds a sent message to the statistics
    pub fn add_sent(&mut self, msg: &NetworkMessage, to_inbound: bool) {
        let d_ref = self.get_data_ref(msg);

        let (to, bytes) = if to_inbound {
            (&mut d_ref.0.to_inbounds, &mut d_ref.1.to_inbounds)
        } else {
            (&mut d_ref.0.to_outbounds, &mut d_ref.1.to_outbounds)
        };

        *to += 1;
        *bytes += msg.get_size() as u32;
    }

    /// Adds a receive message to the statistics
    pub fn add_received(&mut self, msg: &NetworkMessage, from_inbound: bool) {
        let d_ref = self.get_data_ref(msg);

        let (from, bytes) = if from_inbound {
            (&mut d_ref.0.from_inbounds, &mut d_ref.1.from_inbounds)
        } else {
            (&mut d_ref.0.from_outbounds, &mut d_ref.1.from_outbounds)
        };

        *from += 1;
        *bytes += msg.get_size() as u32;
    }

    pub fn get_sent_count(&self) -> u32 {
        self.inv.to_inbounds
            + self.inv.to_outbounds
            + self.get_data.to_inbounds
            + self.get_data.to_outbounds
            + self.tx.to_inbounds
            + self.tx.to_outbounds
            + self.reqrecon.to_inbounds
            + self.reqrecon.to_outbounds
            + self.sketch.to_inbounds
            + self.sketch.to_outbounds
            + self.reconcildiff.to_inbounds
            + self.reconcildiff.to_outbounds
    }

    pub fn get_sent_to_inbounds_count(&self) -> u32 {
        self.inv.to_inbounds
            + self.get_data.to_inbounds
            + self.tx.to_inbounds
            + self.reqrecon.to_inbounds
            + self.sketch.to_inbounds
            + self.reconcildiff.to_inbounds
    }

    pub fn get_sent_to_outbounds_count(&self) -> u32 {
        self.inv.to_outbounds
            + self.get_data.to_outbounds
            + self.tx.to_outbounds
            + self.reqrecon.to_outbounds
            + self.sketch.to_outbounds
            + self.reconcildiff.to_outbounds
    }

    pub fn get_received_count(&self) -> u32 {
        self.inv.from_inbounds
            + self.inv.from_outbounds
            + self.get_data.from_inbounds
            + self.get_data.from_outbounds
            + self.tx.from_inbounds
            + self.tx.from_outbounds
            + self.reqrecon.from_inbounds
            + self.reqrecon.from_outbounds
            + self.sketch.from_inbounds
            + self.sketch.from_outbounds
            + self.reconcildiff.from_inbounds
            + self.reconcildiff.from_outbounds
    }

    pub fn get_received_from_inbounds_count(&self) -> u32 {
        self.inv.from_inbounds
            + self.get_data.from_inbounds
            + self.tx.from_inbounds
            + self.reqrecon.from_inbounds
            + self.sketch.from_inbounds
            + self.reconcildiff.from_inbounds
    }

    pub fn get_received_from_outbounds_count(&self) -> u32 {
        self.inv.from_outbounds
            + self.get_data.from_outbounds
            + self.tx.from_outbounds
            + self.reqrecon.from_outbounds
            + self.sketch.from_outbounds
            + self.reconcildiff.from_outbounds
    }

    pub fn get_sent_bytes(&self) -> u32 {
        self.bytes.to_inbounds + self.bytes.to_outbounds
    }

    pub fn get_sent_to_inbounds_bytes(&self) -> u32 {
        self.bytes.to_inbounds
    }

    pub fn get_sent_to_outbounds_bytes(&self) -> u32 {
        self.bytes.to_outbounds
    }

    pub fn get_received_bytes(&self) -> u32 {
        self.bytes.from_inbounds + self.bytes.from_outbounds
    }

    pub fn get_received_from_outbounds_bytes(&self) -> u32 {
        self.bytes.from_outbounds
    }

    pub fn get_received_from_inbounds_bytes(&self) -> u32 {
        self.bytes.from_inbounds
    }
}

impl Default for NodeStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl Sum for NodeStatistics {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(
            NodeStatistics {
                inv: Data::new(),
                get_data: Data::new(),
                tx: Data::new(),
                reqrecon: Data::new(),
                sketch: Data::new(),
                reconcildiff: Data::new(),
                bytes: Data::new(),
            },
            |a, b| NodeStatistics {
                inv: (a.inv + b.inv),
                get_data: (a.get_data + b.get_data),
                tx: (a.tx + b.tx),
                reqrecon: (a.reqrecon + b.reqrecon),
                sketch: (a.sketch + b.sketch),
                reconcildiff: (a.reconcildiff + b.reconcildiff),
                bytes: a.bytes + b.bytes,
            },
        )
    }
}

pub type AccumulatedStatistics = NodeStatistics;

pub struct AveragedStatistics {
    sent_reachable: f32,
    received_reachable: f32,
    sent_unreachable: f32,
    received_unreachable: f32,
}

impl AveragedStatistics {
    pub fn sent_reachable(&self) -> f32 {
        self.sent_reachable
    }

    pub fn received_reachable(&self) -> f32 {
        self.received_reachable
    }

    pub fn sent_unreachable(&self) -> f32 {
        self.sent_unreachable
    }

    pub fn received_unreachable(&self) -> f32 {
        self.received_unreachable
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
            sent_reachable: self.reachable_stats.get_sent_count() as f32
                / self.reachable_count as f32,
            received_reachable: self.reachable_stats.get_received_count() as f32
                / self.reachable_count as f32,
            sent_unreachable: self.unreachable_stats.get_sent_count() as f32
                / self.unreachable_count as f32,
            received_unreachable: self.unreachable_stats.get_received_count() as f32
                / self.unreachable_count as f32,
        }
    }

    pub fn avg_bytes(&self) -> AveragedStatistics {
        AveragedStatistics {
            sent_reachable: self.reachable_stats.get_sent_bytes() as f32
                / self.reachable_count as f32,
            received_reachable: self.reachable_stats.get_received_bytes() as f32
                / self.reachable_count as f32,
            sent_unreachable: self.unreachable_stats.get_sent_bytes() as f32
                / self.unreachable_count as f32,
            received_unreachable: self.unreachable_stats.get_received_bytes() as f32
                / self.unreachable_count as f32,
        }
    }
}
