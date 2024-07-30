use crate::TxId;

pub type ShortID = u32;

// This is a hack. A sketch is really built using Minisketch. However, this is not necessary for the simulator.
// The only thing we need to know is what transactions a node knows, and what is the size of the difference between that
// and the set of transaction its peer knows. The difference can be computed on the fly, but it is stored here so we can keep
// track of the size of the message for statistics.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Sketch {
    tx_set: Vec<ShortID>,
    q: usize,
}

impl Sketch {
    pub fn new(tx_set: Vec<TxId>, q: usize) -> Self {
        Self { tx_set, q }
    }

    pub fn get_tx_set(&self) -> &Vec<TxId> {
        &self.tx_set
    }

    pub fn get_size(&self) -> usize {
        self.q
    }
}

#[derive(Clone)]
pub struct TxReconciliationState {
    /// Whether this peer is the reconciliation initiator or we are
    pub is_initiator: bool,
    /// Set of transactions to be reconciled with this peer (using Erlay).
    /// Normally, ShortIDs would be 32 bits as opposed to 256-bit TxIds. However, we are already simplifying
    /// TxIds to be 32-bit, so here it'd be a one-to-one mapping
    to_be_reconciled: Vec<ShortID>,
}

impl TxReconciliationState {
    pub fn new(is_initiator: bool) -> Self {
        Self {
            is_initiator,
            to_be_reconciled: Vec::new(),
        }
    }

    pub fn is_initiator(&self) -> bool {
        self.is_initiator
    }

    pub fn add_tx_to_be_reconciled(&mut self, txid: TxId) {
        self.to_be_reconciled.push(txid)
    }
}
