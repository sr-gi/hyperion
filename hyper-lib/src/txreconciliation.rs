use std::collections::HashSet;

use itertools::Itertools;

use crate::TxId;

pub type ShortID = u32;

// This is a hack. A sketch is really built using Minisketch. However, this is not necessary for the simulator.
// The only thing we need to know is what transactions a node knows, and what is the size of the difference between that
// and the set of transaction its peer knows. The difference can be computed on the fly, but it is stored here so we can keep
// track of the size of the message for statistics.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Sketch {
    tx_set: Vec<ShortID>,
    d: usize,
}

impl Sketch {
    pub fn new(tx_set: Vec<TxId>, d: usize) -> Self {
        Self { tx_set, d }
    }

    pub fn get_tx_set(&self) -> &Vec<TxId> {
        &self.tx_set
    }

    pub fn get_size(&self) -> usize {
        self.d
    }
}

#[derive(Clone)]
pub struct TxReconciliationState {
    /// Whether this peer is the reconciliation initiator or we are
    is_initiator: bool,
    /// Whether we are currently reconciling with this peer or not
    is_reconciling: bool,
    /// Set of transactions to be reconciled with this peer (using Erlay).
    /// Normally, ShortIDs would be 32 bits as opposed to 256-bit TxIds. However, we are already simplifying
    /// TxIds to be 32-bit, so here it'd be a one-to-one mapping
    recon_set: HashSet<ShortID>,
    // Set of transactions to be added to the reconciliation set on the next trickle. These are still unrequestable for
    // privacy reasons (to prevent transaction proving), transactions became available once they would have been announced
    // via fanout (on the next trickle).
    delayed_set: HashSet<ShortID>,
}

impl TxReconciliationState {
    pub fn new(is_initiator: bool) -> Self {
        Self {
            is_initiator,
            is_reconciling: false,
            recon_set: HashSet::new(),
            delayed_set: HashSet::new(),
        }
    }

    pub fn clear(&mut self) -> HashSet<ShortID> {
        self.is_reconciling = false;
        self.recon_set.drain().collect()
    }

    pub fn is_initiator(&self) -> bool {
        self.is_initiator
    }

    pub fn add_tx(&mut self, txid: TxId) -> bool {
        self.delayed_set.insert(txid)
    }

    /// Removes a transaction from the reconciliation set. This may happen if a peer has announced a transaction that we
    /// were planing to reconcile with them. Notice that, if this happens after creating a snapshot, the reconciliation will
    /// result in one additional item on the exchanged INV (belonging to this transaction). This is equivalent to two INVs crossing,
    /// and AFAIK, there's nothing we can do about it
    pub fn remove_tx(&mut self, txid: &TxId) {
        self.delayed_set.remove(txid);
        self.recon_set.remove(txid);
    }

    // Make delayed transactions available for reconciliation
    pub fn make_delayed_available(&mut self) {
        self.recon_set.extend(self.delayed_set.drain());
    }

    pub fn set_reconciling(&mut self) {
        self.is_reconciling = true;
    }

    pub fn is_reconciling(&self) -> bool {
        self.is_reconciling
    }

    pub fn get_recon_set(&self) -> &HashSet<ShortID> {
        &self.recon_set
    }

    pub fn compute_sketch(&mut self, their_txs: Vec<TxId>) -> Sketch {
        // Given predicting q is hard in a short simulation, we exchange the collection of transactions to be reconciled
        // between the two parties. Here, q is computed as the difference between the two collections (local and remote)
        // And a figurative Sketch is created
        let local_set = self.get_recon_set();
        let remote_set = HashSet::from_iter(their_txs);
        let q = local_set.symmetric_difference(&remote_set).count();
        // TODO: Scale q if required so the predicted difference is not always 100% accurate
        Sketch::new(local_set.iter().copied().collect_vec(), q)
    }

    pub fn compute_sketch_diff(&self, sketch: Sketch) -> (Vec<u32>, Vec<u32>) {
        let local_set: HashSet<&u32> = HashSet::from_iter(self.get_recon_set());
        let remote_set = HashSet::from_iter(sketch.get_tx_set());

        // We care about what we are missing, that's what we send to our peer
        let our_diff: Vec<u32> = remote_set.difference(&local_set).map(|x| **x).collect_vec();
        let their_diff = local_set.difference(&remote_set).map(|x| **x).collect_vec();

        (our_diff, their_diff)
    }
}
