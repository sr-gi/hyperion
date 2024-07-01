use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::thread_rng;
use rand_distr::{Distribution, Poisson};

use crate::network::NetworkMessage;
use crate::TxId;

pub type NodeId = u32;

static INBOUND_INVENTORY_BROADCAST_INTERVAL: u16 = 5;
static OUTBOUND_INVENTORY_BROADCAST_INTERVAL: u16 = 2;
static NONPREF_PEER_TX_DELAY: u16 = 2;

#[derive(Clone)]
pub struct Peer {
    known_transactions: HashSet<TxId>,
    inv_queue: VecDeque<NetworkMessage>,
}

impl Peer {
    pub fn new() -> Self {
        Peer {
            known_transactions: HashSet::new(),
            inv_queue: VecDeque::new(),
        }
    }

    fn knows_transactions(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    fn schedule_inv(&mut self, msg: NetworkMessage) {
        assert!(msg.is_inv());
        self.inv_queue.push_back(msg)
    }
}

#[derive(Clone)]
pub struct Node {
    node_id: NodeId,
    is_reachable: bool,
    in_peers: HashMap<NodeId, Arc<Mutex<Peer>>>,
    out_peers: HashMap<NodeId, Arc<Mutex<Peer>>>,
    wire: Sender<(NetworkMessage, NodeId, NodeId)>,
    requested_transactions: Arc<Mutex<HashSet<TxId>>>,
    delayed_requests: Arc<Mutex<HashMap<TxId, (NodeId, Instant)>>>,
    known_transactions: HashSet<TxId>,
}

impl Node {
    pub fn new(
        node_id: NodeId,
        is_reachable: bool,
        wire: Sender<(NetworkMessage, NodeId, NodeId)>,
    ) -> Self {
        Node {
            node_id,
            is_reachable,
            in_peers: HashMap::new(),
            out_peers: HashMap::new(),
            wire,
            requested_transactions: Arc::new(Mutex::new(HashSet::new())),
            delayed_requests: Arc::new(Mutex::new(HashMap::new())),
            known_transactions: HashSet::new(),
        }
    }

    pub fn dispatch_invs(&self) {
        let node_id = self.node_id;

        let inbounds_poisson = Poisson::new(INBOUND_INVENTORY_BROADCAST_INTERVAL as f32).unwrap();
        let outbounds_poisson = Poisson::new(OUTBOUND_INVENTORY_BROADCAST_INTERVAL as f32).unwrap();

        let wire = self.wire.clone();
        let in_peers = self.in_peers.clone();
        std::thread::spawn(move || loop {
            // All inbounds are on the same timer
            let mut inbounds_timer = thread_rng();
            let next_interval = inbounds_poisson.sample(&mut inbounds_timer);
            log::trace!(
                "(Node {}) Next inv processing time in {next_interval}s",
                node_id
            );
            std::thread::sleep(Duration::from_secs_f32(next_interval));

            for (peer_id, peer) in in_peers.iter() {
                while !peer.lock().unwrap().inv_queue.is_empty() {
                    wire.send((
                        peer.lock().unwrap().inv_queue.pop_front().unwrap(),
                        node_id,
                        *peer_id,
                    ))
                    .unwrap();
                }
            }
        });

        for (peer_id, peer) in self.out_peers.iter() {
            let peer = peer.clone();
            let peer_id = peer_id.clone();
            let wire = self.wire.clone();
            std::thread::spawn(move || loop {
                // Outbounds have a timer on their own
                let next_interval = outbounds_poisson.sample(&mut thread_rng());
                log::trace!(
                    "(Node {}) Next inv processing time in {next_interval}s",
                    node_id
                );
                std::thread::sleep(Duration::from_secs_f32(next_interval));

                let inv_queue = &mut peer.lock().unwrap().inv_queue;
                while !inv_queue.is_empty() {
                    wire.send((inv_queue.pop_front().unwrap(), node_id, peer_id))
                        .unwrap();
                }
            });
        }
    }

    pub fn handle_delayed_requests(&self) {
        let node_id = self.node_id;
        let delayed_requests = self.delayed_requests.clone();
        let requested_transactions = self.requested_transactions.clone();
        let wire = self.wire.clone();
        std::thread::spawn(move || loop {
            // Delay requests for NONPREF_PEER_TX_DELAY to give outbound peers priority.
            let timeout = Duration::from_secs(NONPREF_PEER_TX_DELAY as u64);
            std::thread::sleep(timeout);

            let mut to_remove = Vec::new();
            for (txid, (peer_id, delay)) in delayed_requests.lock().unwrap().iter() {
                if delay.elapsed() > timeout {
                    to_remove.push((*txid, *peer_id));
                }
            }

            let mut locked_delayed = delayed_requests.lock().unwrap();
            let mut locked_requests = requested_transactions.lock().unwrap();
            for (txid, peer_id) in to_remove.iter() {
                locked_delayed.remove(txid);
                // If no other request have been made, move the delayed request to requests
                if !locked_requests.contains(txid) {
                    locked_requests.insert(*txid);
                }

                wire.send((NetworkMessage::GETDATA(*txid), node_id, *peer_id))
                    .unwrap();
            }
        });
    }

    fn get_peer(&self, peer_id: &NodeId) -> Option<&Arc<Mutex<Peer>>> {
        self.out_peers
            .get(&peer_id)
            .or_else(|| self.in_peers.get(peer_id))
    }

    pub fn get_id(&self) -> NodeId {
        self.node_id
    }

    pub fn get_inbounds(&self) -> &HashMap<NodeId, Arc<Mutex<Peer>>> {
        &self.in_peers
    }

    pub fn get_outbounds(&self) -> &HashMap<NodeId, Arc<Mutex<Peer>>> {
        &self.out_peers
    }

    pub fn is_inbounds(&self, peer_id: &NodeId) -> bool {
        self.in_peers.contains_key(&peer_id)
    }

    pub fn connect(&mut self, peer_id: NodeId) {
        assert!(
            !self.in_peers.contains_key(&peer_id),
            "Peer {peer_id} is already connected to us"
        );
        assert!(
            self.out_peers
                .insert(peer_id, Arc::new(Mutex::new(Peer::new())))
                .is_none(),
            "We ({}) are already connected to {peer_id}",
            self.node_id
        );
    }

    pub fn accept_connection(&mut self, peer_id: NodeId) {
        assert!(
            self.is_reachable,
            "Node {peer_id} tried to connect to us (node_id: {}), but we are not reachable",
            self.node_id
        );
        assert!(
            !self.out_peers.contains_key(&peer_id),
            "We (node_id: {}) are already connected to peer {peer_id}",
            self.node_id
        );
        assert!(
            self.in_peers
                // Inbounds are all on the same shared timer
                .insert(peer_id, Arc::new(Mutex::new(Peer::new())))
                .is_none(),
            "Peer {peer_id} is already connected to us"
        );
    }

    fn knows_transactions(&self, txid: &TxId) -> bool {
        self.known_transactions.contains(txid)
    }

    fn add_known_transaction(&mut self, txid: TxId) {
        self.known_transactions.insert(txid);
    }

    pub fn broadcast_tx(&mut self, txid: TxId) {
        self.add_known_transaction(txid);
        let inv = NetworkMessage::INV(txid);

        let mut peers: Vec<u32> = self.in_peers.keys().cloned().collect();
        peers.extend(self.out_peers.keys());
        for peer_id in peers {
            self.send_message_to(inv, peer_id)
        }
    }

    fn add_request(&self, txid: TxId, peer_id: NodeId) -> bool {
        // Transactions are only requested from a single peer (assuming honest behavior)
        // Inbound peers are de-prioritized. If an outbound peer announces a transaction
        // and an inbound peer request is in delayed stage, the inbounds will be dropped and
        // the outbound will be processed
        let mut delayed_requests = self.delayed_requests.lock().unwrap();
        let mut requested_transactions = self.requested_transactions.lock().unwrap();

        if !self.knows_transactions(&txid) && !requested_transactions.contains(&txid) {
            if self.is_inbounds(&peer_id) {
                if !delayed_requests.contains_key(&txid) {
                    delayed_requests.insert(txid, (peer_id, Instant::now()));
                    log::info!(
                        "(Node {}) Delaying getdata for transaction request (txid: {txid:x}) to peer {peer_id}",
                        self.node_id
                    );
                }
            } else {
                requested_transactions.insert(txid);
                delayed_requests.remove(&txid);
                return true;
            }
        } else {
            log::info!(
                "(Node {}) Already known transaction (txid: {txid:x}), not requesting to (peer_id: {peer_id})",
                self.node_id
            )
        }
        return false;
    }

    fn remove_request(&self, txid: TxId) {
        let mut requested_transactions = self.requested_transactions.lock().unwrap();
        let delayed_requests = self.delayed_requests.lock().unwrap();
        assert!(!delayed_requests.contains_key(&txid));
        assert!(requested_transactions.remove(&txid));
    }

    pub fn send_message_to(&mut self, msg: NetworkMessage, peer_id: NodeId) {
        if let Some(peer) = self.get_peer(&peer_id) {
            let mut peer = peer.lock().unwrap();
            let txid = *msg.inner();
            let we_know_tx = self.knows_transactions(&txid);
            if msg.is_inv() {
                // INVs for the same transaction can get crossed. If this happens break the cycle.
                assert!(we_know_tx, "Trying to announce a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                if peer.knows_transactions(&txid) {
                    log::debug!(
                        "(Node {}) We already know about transaction {txid:x}, not requesting it back to {}",
                        self.node_id,
                        peer_id
                    );
                    return;
                }
                peer.schedule_inv(msg);
                log::info!("(Node {}) Scheduling {msg} to peer {peer_id}", self.node_id);
            } else if msg.is_get_data() {
                assert!(!we_know_tx, "Trying to request a transaction we already know about (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                assert!(peer.knows_transactions(&txid), "Trying to request a transaction (txid: {txid:x}) from a peer that shouldn't know about it (peer_id: {peer_id})");

                if !self.add_request(txid, peer_id) {
                    return;
                }
                self.wire.send((msg, self.node_id, peer_id)).unwrap();
                log::info!("(Node {}) Sending {msg} to peer {peer_id}", self.node_id);
            } else {
                assert!(we_know_tx, "Trying to send a transaction we should't know about (txid: {txid:x}) to a peer (peer_id: {peer_id})");
                assert!(!peer.knows_transactions(&txid), "Trying to send a transaction (txid: {txid:x}) to a peer that already should know about it (peer_id: {peer_id})");
                self.wire.send((msg, self.node_id, peer_id)).unwrap();
                peer.add_known_transaction(txid);
                log::info!("(Node {}) Sending {msg} to peer {peer_id}", self.node_id);
            }
        }
    }

    pub fn receive_message_from(&mut self, msg: NetworkMessage, peer_id: NodeId) {
        let mut response = None;

        log::info!("(Node {}) Received {msg} from peer {peer_id}", self.node_id);
        if let Some(peer) = self.get_peer(&peer_id) {
            let mut peer = peer.lock().unwrap();
            let txid = *msg.inner();
            let we_know_tx = self.knows_transactions(&txid);
            if msg.is_inv() {
                peer.add_known_transaction(txid);
                // We only request transactions that we don't know about
                if !we_know_tx {
                    log::info!(
                        "(Node {}) Transaction unknown. Scheduling getdata to peer {peer_id}",
                        self.node_id
                    );
                    response = Some((NetworkMessage::GETDATA(txid), peer_id));
                } else {
                    log::info!(
                        "(Node {}) Already known transaction (txid: {txid:x})",
                        self.node_id
                    );
                }
            } else if msg.is_get_data() {
                assert!(we_know_tx, "Received transaction request for a transaction we don't know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                assert!(!peer.knows_transactions(&txid), "Received a transaction request (txid: {txid:x}) from a peer that should already know about it (peer_id {peer_id})");
                response = Some((NetworkMessage::TX(txid), peer_id));
            } else {
                assert!(!we_know_tx, "Received a transaction we already know of (txid: {txid:x}) from a peer (peer_id: {peer_id})");
                assert!(peer.knows_transactions(&txid), "Received a transaction (txid: {txid:x}) from a node that shouldn't know about it (peer_id {peer_id})");
                self.remove_request(txid);
            }
        }

        if let Some((msg, peer_id)) = response {
            self.send_message_to(msg, peer_id)
        }

        // The simulator makes sure a transaction is received by a node twice.
        // Therefore, receiving one is a trigger to start broadcasting.
        // Notice we cannot do this while holding a peer reference
        if msg.is_tx() {
            self.broadcast_tx(*msg.inner());
        }
    }
}
