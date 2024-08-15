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

#[cfg(test)]
mod test {
    use super::*;
    use crate::test::get_random_txid;

    #[test]
    fn test_recon_state() {
        let mut tx_recon_state = TxReconciliationState::new(true);
        tx_recon_state.set_reconciling();
        assert!(tx_recon_state.is_reconciling());
        assert!(tx_recon_state.recon_set.is_empty());
        assert!(tx_recon_state.delayed_set.is_empty());

        // Add some txs to the recon set
        for _ in 0..10 {
            tx_recon_state.add_tx(get_random_txid());
        }

        // Check that all transactions are delayed
        assert!(tx_recon_state.recon_set.is_empty());
        assert!(!tx_recon_state.delayed_set.is_empty());

        // Move to available and check again
        tx_recon_state.make_delayed_available();
        assert!(!tx_recon_state.recon_set.is_empty());
        assert!(tx_recon_state.delayed_set.is_empty());

        // Add some more transactions to delayed
        for _ in 0..10 {
            tx_recon_state.add_tx(get_random_txid());
        }
        assert!(!tx_recon_state.recon_set.is_empty());
        assert!(!tx_recon_state.delayed_set.is_empty());

        let txs = tx_recon_state.get_recon_set().clone();
        let delayed_txs = tx_recon_state.delayed_set.clone();

        // Clear and  check that the set returned is the one before clearing
        // that the recon set is empty, but the delayed set is preserved
        // and that the state is set as not reconciling anymore
        assert_eq!(tx_recon_state.clear(), txs);
        assert_eq!(tx_recon_state.delayed_set, delayed_txs);
        assert!(tx_recon_state.recon_set.is_empty());
        assert!(!tx_recon_state.delayed_set.is_empty());
        assert!(!tx_recon_state.is_reconciling());
    }

    #[test]
    fn test_sketch() {
        let mut tx_recon_state = TxReconciliationState::new(true);
        let mut our_txs = Vec::new();
        let mut their_txs = Vec::new();
        let mut unknown_by_us = Vec::new();
        let mut unknown_by_them = Vec::new();
        let d = 8;

        // Add some txs to the recon set
        for i in 0..10 {
            let txid = get_random_txid();
            tx_recon_state.recon_set.insert(txid);
            our_txs.push(txid);

            // Make half of the transactions also known by our peer (5)
            if i % 2 == 0 {
                their_txs.push(txid);
            } else {
                unknown_by_them.push(txid)
            }
        }

        // Create some transactions (3) unknown by us and add them to their set
        for _ in 0..3 {
            let txid = get_random_txid();
            unknown_by_us.push(txid);
            their_txs.push(txid);
        }

        // Sort the vectors so they can be properly compared later
        our_txs.sort();
        their_txs.sort();
        unknown_by_us.sort();
        unknown_by_them.sort();

        let our_sketch = tx_recon_state.compute_sketch(their_txs.clone());
        let mut sketch_txs = our_sketch.get_tx_set().clone();
        sketch_txs.sort();

        // Check that our sketch contains the transactions we added to it
        // and that it doesn't contain any of the transactions that only they had
        assert_eq!(sketch_txs, our_txs);
        for tx in their_txs.iter() {
            if unknown_by_us.contains(tx) {
                assert!(!our_sketch.get_tx_set().contains(tx))
            }
        }
        // The set difference is 8: 5 (they're missing) + 3 (we're missing)
        assert_eq!(our_sketch.get_size(), d);

        // Compute their sketch and diff them
        let their_sketch = Sketch {
            tx_set: their_txs.clone(),
            d,
        };
        let (mut our_diff, mut their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        our_diff.sort();
        their_diff.sort();
        assert_eq!(our_diff, unknown_by_us);
        assert_eq!(their_diff, unknown_by_them);
    }
}
