# Hyperion

Hyperion is a discrete time network event simulator for Bitcoin. It's goal is to simulate how data propagates over a simulated network using different techniques.

The motivation behind the project is being able to compare distinct transaction relaying techniques, and profile design parameter for them. An example would be finding the ratio between fanout and reconciling peers for Erlay.

## Design

The simulator is event based, with a global priority queue that takes care of processing the events.

A simulation currently consist in sending a single transaction from a randomly selected peer in the network to all other nodes, and is completed when all messages have been exchanged.

Nodes in a simulator are well behaved, that is, all nodes follow the transaction announcement flow, they do not bloat the network nor spam other nodes, transactions are not sent unrequested, requests are always replied to, etc.

The transaction relaying logic follows Bitcoin Core's design:

- `INVs` are sent to peers at random intervals based on whether a peer is inbound or outbound:
    - For inbounds, the delay follows a Poisson process with an expected value of `INBOUND_INVENTORY_BROADCAST_INTERVAL(5s)`. All inbounds are on the same timer
    - For outbounds, the delay follows a Poisson process with an expected value of `OUTBOUND_INVENTORY_BROADCAST_INTERVAL(2s)`. Every outbound has a unique timer
- `GETDATA`s are prioritized to outbound peers, hence if an inbound peer announces a transaction, the request will be delayed by `NONPREF_PEER_TX_DELAY(2s)`, and superseded by any other request by an outbound peer<sup>1</sup>
- `REQRECON` messages are sent on a fix timer, in a round-robin fashion every `RECON_REQUEST_INTERVAL(8s)`, meaning each request will go out every `8/n`s where `n` is the outbound peer count of the node
- `SKETCH`es exchanged between peers contain transactions as long as those would have been requestable by the peer at the time of sharing the sketch (this means if an `INV` containing such transaction would have been created)

All messages exchanged between peers are added some network latency, which is sampled at random from a Log Normal distribution with expected value of `10ms` and variance of `2ms`.

<sup>1</sup> Notice `GETDATA` requests are not queued, given nodes are well behaved, a node announcing a transaction and getting a `GETDATA` request back will always reply with the corresponding transaction.

## Status of the project

The simulator implements the traditional `INV -> GETDATA -> TX` (fanout) and the Erlay logic without reconciliation extensions (given sets are always perfectly reconstructed). A simulation consist of sending a single transaction from a randomly selected node and executing the corresponding events until all messages have been exchanged.

## Usage
```
Usage: hyperion [OPTIONS]

Options:
  -r, --reachable <REACHABLE>
          The number of reachable nodes in the simulated network [default: 10000]
  -u, --unreachable <UNREACHABLE>
          The number of unreachable nodes in the simulated network [default: 100000]
  -o, --outbounds <OUTBOUNDS>
          The number of outbound connections established per node [default: 8]
  -l, --log-level <LOG_LEVEL>
          Level of verbosity of the messages displayed by the simulator.
          Possible values: [off, error, warn, info, debug, trace] [default: info]
  -p, --percentile-target <PERCENTILE_TARGET>
          Propagation percentile target. Use to measure transaction propagation times [default: 90]
      --erlay
          Whether or not nodes in the simulation support Erlay (all of them for now, this is likely to change)
  -s, --seed <SEED>
          Seed to run random activity generator deterministically
      --no-latency
          Don't add network latency to messages exchanges between peers. Useful for debugging
  -n, --n <N>
          Number of times the simulation will be repeated. Smooths the statistical results [default: 1]
  -h, --help
          Print help
  -V, --version
          Print version
  ```