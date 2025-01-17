#!/bin/bash

# Help function
usage() {
    echo "Usage: $0 [-n <n>] [--out=<out>] [--outbounds=<outbounds>] [--seed=<seed>] [--ipm=<ipm>] [--opm=<opm>] [-h|--help]"
    echo
    echo "Options:"
    echo "  -n                  Number of times the simulation will be repeated. Smooths the statistical results [default: 1]"
    echo "  --out               output file name (default: result.csv)"
    echo "  --outbounds         Number of outbound connections per node [default: 8]"
    echo "  --seed              RNG seed for reproducible results. Randomly generated if not provided"
    echo "  --ipm               Inbounds Poisson timer mean value [default: 5]"
    echo "  --opm               Outbounds Poisson timer mean value [default: 2]"
    echo "  -h | --help         Show this help message and exit"
    exit 1
}

CALL_PARAMS=""
OUTPUT_FILE="result.csv"
N=100

# Parse arguments
for arg in "$@"; do
    case $arg in
        -n=*)
            N="${arg#*=}" ;;
        --out=*)
            OUTPUT_FILE="${arg#*=}" ;;
        --outbounds=*)
            CALL_PARAMS+="--outbounds ${arg#*=} " ;;
        --seed=*)
            CALL_PARAMS+="--seed ${arg#*=} " ;;
        --ipm=*)
            INBOUND_INVENTORY_BROADCAST_INTERVAL="${arg#*=}"
            export INBOUND_INVENTORY_BROADCAST_INTERVAL ;;
        --opm=*)
            OUTBOUND_INVENTORY_BROADCAST_INTERVAL="${arg#*=}"
            export OUTBOUND_INVENTORY_BROADCAST_INTERVAL ;;
        -h |--help)
            usage ;;
    *)
      echo "Unknown option: $arg"
      usage ;;
  esac
done

# Iter options
MAX_OUTBOUNDS=8
MAX_INBOUNDS_FRACTION=1.0
INBOUND_INCREMENT=0.1

for ((OUTBOUND_FANOUT_DESTINATIONS = 0; OUTBOUND_FANOUT_DESTINATIONS <= MAX_OUTBOUNDS; OUTBOUND_FANOUT_DESTINATIONS++))
do
    export OUTBOUND_FANOUT_DESTINATIONS
    for INBOUND_FANOUT_DESTINATIONS_FRACTION in $(seq 0.0 $INBOUND_INCREMENT $MAX_INBOUNDS_FRACTION)
    do
        export INBOUND_FANOUT_DESTINATIONS_FRACTION
        cargo run --release -- --erlay -n $N --out $OUTPUT_FILE $CALL_PARAMS
    done
done