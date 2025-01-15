#!/bin/bash

# PARAMS
$CALL_PARAMS = ""
[[ -n "$1" ]] && CALL_PARAMS+="--outbounds $1 "
[[ -n "$2" ]] && CALL_PARAMS+="--seed $2 "
if [[ -n "$3" ]]; then CALL_PARAMS+="--out $3"
elif [[ -n "$1" ]]; then CALL_PARAMS+="--out result_"$1".csv"
else CALL_PARAMS+="--out result.csv"
fi

# ITER OPTIONS
MAX_OUTBOUNDS=8
MAX_INBOUNDS_FRACTION=1.0
INBOUND_INCREMENT=0.1

for ((OUTBOUND_FANOUT_DESTINATIONS = 0; OUTBOUND_FANOUT_DESTINATIONS < MAX_OUTBOUNDS; OUTBOUND_FANOUT_DESTINATIONS++))
do
    export OUTBOUND_FANOUT_DESTINATIONS
    for INBOUND_FANOUT_DESTINATIONS_FRACTION in $(seq 0.0 $INBOUND_INCREMENT $MAX_INBOUNDS_FRACTION)
    do
        export INBOUND_FANOUT_DESTINATIONS_FRACTION
        cargo run --release -- --erlay -n 100 $CALL_PARAMS
    done
done