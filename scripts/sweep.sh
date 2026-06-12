#!/usr/bin/env bash
#
# Sweep worker count x loading mode (per-task work) x global-queue share,
# writing one JSON object per run to data/grid.jsonl (throughput + queue
# metrics). `share = 0` is the legacy 1/N behaviour (knob unset).
#
# Override with WORKERS=..., SPINS=..., SHARES=..., OPS=..., PRODUCERS=..., TRIALS=...
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --bin flood
B=target/release/flood
WORKERS="${WORKERS:-8 16 32 64 128}"
SPINS="${SPINS:-0 4000 64000}"          # 0 = scheduling-bound, 4000 ~ mixed, 64000 ~ work-bound
SHARES="${SHARES:-0 0.25 0.5 0.75 1.0}" # 0 => legacy 1/N (knob unset)
OPS="${OPS:-5000000}"
P="${PRODUCERS:-24}"
TRIALS="${TRIALS:-3}"

mkdir -p data
: > data/grid.jsonl
for W in $WORKERS; do
  for SPIN in $SPINS; do
    for SH in $SHARES; do
      for _ in $(seq 1 "$TRIALS"); do "$B" "$W" "$SH" "$P" "$OPS" "$SPIN" | tee -a data/grid.jsonl; done
    done
  done
done

echo
echo "Render: python3 scripts/plot.py"
echo "Aggregate, e.g.: python3 scripts/aggregate.py data/grid.jsonl workers throughput_ops_per_s spin=0"
