pub type ShortID = u32;

/// This is a hack. A sketch is really built using Minisketch. However, this is not necessary for the simulator.
/// The only thing we need to know is whether the node knows the transaction, and what is the size of the difference
/// between that and the set of transaction its peer knows (1 or 0, given we are simulating a single transactions).
/// The difference can be computed on the fly, but it is stored here so we can keep
/// track of the size of the message for statistics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Sketch {
    tx_set: bool,
    d: usize,
}

impl Sketch {
    pub fn new(tx_set: bool, d: usize) -> Self {
        Self { tx_set, d }
    }

    pub fn get_tx_set(&self) -> bool {
        self.tx_set
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
    /// Whether the simulated transaction is in the reconciliation set
    recon_set: bool,
}

impl TxReconciliationState {
    pub fn new(is_initiator: bool) -> Self {
        Self {
            is_initiator,
            is_reconciling: false,
            recon_set: false,
        }
    }

    pub fn clear(&mut self) -> bool {
        let recon_set = self.recon_set;
        self.is_reconciling = false;
        self.recon_set = false;

        recon_set
    }

    pub fn is_initiator(&self) -> bool {
        self.is_initiator
    }

    pub fn add_tx(&mut self) -> bool {
        let r = !self.recon_set;
        self.recon_set = true;

        r
    }

    /// Removes the transaction from the reconciliation set. This may happen if a peer has announced the transaction that we
    /// were planing to reconcile with them. Notice that, if this happens after creating a snapshot, the reconciliation will
    /// result in one additional INV (belonging to this transaction). This is equivalent to two INVs crossing, and AFAIK,
    /// there's nothing we can do about it
    pub fn remove_tx(&mut self) {
        self.recon_set = false;
    }

    pub fn set_reconciling(&mut self) {
        self.is_reconciling = true;
    }

    pub fn is_reconciling(&self) -> bool {
        self.is_reconciling
    }

    pub fn get_recon_set(&self) -> bool {
        self.recon_set
    }

    pub fn compute_sketch(&self, they_know_tx: bool) -> Sketch {
        // q cannot be easily predicted in a short simulation, however it is needed to size the sketch properly.
        // As a workaround, the sketches exchanges by the simulator are not really sketches, but knowledge of whether
        // the sender knows the given transaction. q can be scaled down if needed to mimic scenarios where the sketch
        // exchange is not perfectly efficient
        let local_set = self.get_recon_set();
        let remote_set = they_know_tx;
        // We can compute the size of the diff as int(A XOR B)
        // TODO: Scale q if required so the predicted difference is not always 100% accurate
        let q = (local_set ^ remote_set) as usize;
        Sketch::new(local_set, q)
    }

    pub fn compute_sketch_diff(&self, sketch: Sketch) -> (bool, bool) {
        let local_set = self.get_recon_set();
        let remote_set = sketch.get_tx_set();

        // A XOR B to see if there is a diff
        let diff = local_set ^ remote_set;
        // If there us a diff, each end's diff is whether the other end knows the transaction
        let local_diff = diff && remote_set;
        let remote_diff = diff && local_set;

        (local_diff, remote_diff)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_recon_state() {
        let mut tx_recon_state = TxReconciliationState::new(true);
        tx_recon_state.set_reconciling();
        assert!(tx_recon_state.is_reconciling());
        assert!(!tx_recon_state.recon_set);

        // Add a transaction to the recon_set
        tx_recon_state.add_tx();

        // Check that the transaction has been added to the recon set
        assert!(tx_recon_state.recon_set);
        assert!(tx_recon_state.is_reconciling());

        // Clear, and check that the set is empty
        tx_recon_state.clear();
        assert!(!tx_recon_state.recon_set);
        assert!(!tx_recon_state.is_reconciling());
    }

    #[test]
    fn test_sketch() {
        // Start from an empty state
        let mut tx_recon_state = TxReconciliationState::new(true);

        // Create their sketch without the transaction. Since none of us know the transaction, the diff size will be 0
        let mut diff_size = 0;
        let mut their_sketch = Sketch::new(false, diff_size);
        assert!(their_sketch.get_size() == diff_size);

        // Compute the diffs and check. None of us know the transaction so the diff should be false
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(!our_diff);
        assert!(!their_diff);

        // Add the tx to the recon set
        tx_recon_state.add_tx();

        // Change their sketch, since now the difference will be 1
        diff_size = 1;
        their_sketch = Sketch::new(false, diff_size);
        assert!(their_sketch.get_size() == diff_size);

        // Compute the diffs and check. We know the tx and they don't, so our diff should be false and theirs should be true
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(!our_diff);
        assert!(their_diff);

        // Update it so now we don't know but they do
        tx_recon_state.clear();
        their_sketch = Sketch::new(true, diff_size);
        assert!(their_sketch.get_size() == diff_size);

        // Compute the diffs and check. They know the transaction and we don't, so out diff should be true and theirs should be false
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(our_diff);
        assert!(!their_diff);

        // Update it so both of us know the transaction
        diff_size = 0;
        tx_recon_state.add_tx();
        their_sketch = Sketch::new(true, diff_size);

        // Compute the diffs and check. We both know the transaction, so both diff should be false
        let (our_diff, their_diff) = tx_recon_state.compute_sketch_diff(their_sketch);
        assert!(!our_diff);
        assert!(!their_diff);
    }
}
