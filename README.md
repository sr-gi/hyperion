# Hyperion

Hyperion is a discrete time network event simulator for Bitcoin. It's goal is to simulate how data propagates throughout a simulated network using different techniques.

The motivation behind the project is being able to compare distinct transaction relaying techniques, and profile design parameter for them. An example would be finding the ratio between fanout and reconciling peers for Erlay.

## Design

The simulator is event based, with a global priority queue that takes care of processing the events.

The simulation currently consist in sending a single transaction from a randomly selected peer in the network to all other nodes, and is completed when all messages have been exchanged.

Nodes in the simulator are well behaved, that is, all nodes follow the transaction announcement flow, they do not bloat the network nor spam other nodes, transactions are not sent unrequested, requests are always replied to, etc.

The node transaction relaying logic follows Bitcoin Core's design:

- `INVs` are sent to peers on random intervals based on whether a peer is inbound or outbound:
    - For inbounds, the delay follows a Poisson process with an expected value of `INBOUND_INVENTORY_BROADCAST_INTERVAL(5s)`. All inbounds are on the same timer
    - For outbounds, the delay follows a Poisson process with an expected value of `OUTBOUND_INVENTORY_BROADCAST_INTERVAL(2s)`. Every outbound has a unique timer
- `GETDATA`s are prioritized to outbound peers, hence if an inbound peer announces a transaction, the request will be
delayed by `NONPREF_PEER_TX_DELAY(2s)`, and superseded by any other request by an outbound peer<sup>1</sup>

All messages exchanged between peers are added some network latency, which is sampled at random from a Log Normal distribution with expected value of `10ms` and variance of `2ms`.

<sup>1</sup> Notice `GETDATA` requests are not queued, given nodes are well behaved, a node announcing a transaction and getting a get data request back will always reply with the corresponding transaction.

## Status of the project

Currently, the simulator only implements the traditional `INV -> GETDATA -> TX` logic for exchanging transactions between peers. The simulation consist on sending a single transaction from a randomly selected node executing the corresponding events until all messages have been exchanged.

## Usage
```
Usage: hyperion [OPTIONS]

Options:
  -r, --reachable <REACHABLE>
          The number of reachable nodes in the simulated network [default: 10000]
  -u, --unreachable <UNREACHABLE>
          The number of unreachable nodes in the simulated network [default: 100000]
  -n, --num-outbounds <NUM_OUTBOUNDS>
          The number of outbound connections established per node [default: 8]
  -l, --log-level <LOG_LEVEL>
          Level of verbosity of the messages displayed by the simulator.
          Possible values: [off, error, warn, info, debug, trace] [default: info]
  -p, --percentile-target <PERCENTILE_TARGET>
          Target percentile of node the transaction needs to reach. Use to measure propagation times [default: 90]
  -s, --seed <SEED>
          Seed to run random activity generator deterministically
  -h, --help
          Print help
  -V, --version
          Print version
  ```