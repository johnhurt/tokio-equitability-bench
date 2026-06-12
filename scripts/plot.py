#!/usr/bin/env python3
"""Render the throughput grid (data/grid.jsonl) to SVG charts in charts/.

    python3 scripts/plot.py [grid.jsonl] [out_dir]

Each line is one `flood` run at a (workers, spin, share) point. `share = 0` is
the legacy `1 / N` behaviour (the knob unset). Cells are medians across trials.

Writes:
    charts/headline.svg    - the "mixed" (~4 us/task) loading mode: throughput
                             vs share, one line per worker count
    charts/worker_<N>.svg  - one chart per worker count, a line per loading mode
"""
import json
import sys
import statistics
from collections import defaultdict

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

GRID = sys.argv[1] if len(sys.argv) > 1 else "data/grid.jsonl"
OUT = sys.argv[2] if len(sys.argv) > 2 else "charts"

WORKERS = [8, 16, 32, 64, 128]
MODES = [(0, "scheduling-bound"), (4000, "mixed (~4 us)"), (64000, "work-bound (~64 us)")]
MODE_COLORS = ["#1f77b4", "#d62728", "#2ca02c"]  # blue, red, green
HEADLINE_SPIN = 4000
SHARES = [0, 0.25, 0.5, 0.75, 1.0]
SHARE_LABELS = ["legacy", "0.25", "0.5", "0.75", "1.0"]
YMAX = 900

samples = defaultdict(list)
for line in open(GRID):
    line = line.strip()
    if not line.startswith("{"):
        continue
    j = json.loads(line)
    samples[(j["workers"], j["spin"], j["share"])].append(j["throughput_ops_per_s"] / 1000.0)


def med(w, sp, s):
    return statistics.median(samples[(w, sp, s)])


# Headline: mixed loading mode, one line per worker count.
fig, ax = plt.subplots(figsize=(7, 4.5))
for w in WORKERS:
    ax.plot(SHARE_LABELS, [med(w, HEADLINE_SPIN, s) for s in SHARES], marker="o", label=f"{w} workers")
ax.set_xlabel("global_queue_share_per_worker")
ax.set_ylabel("throughput (k ops/s)")
ax.set_title("Mixed workload (~4 us/task): throughput vs share\n(higher share -> bigger pulls -> higher throughput)")
ax.grid(True, alpha=0.3)
ax.set_ylim(0, YMAX)
ax.legend(title="workers")
fig.tight_layout()
fig.savefig(f"{OUT}/headline.svg", bbox_inches="tight")
plt.close(fig)

# One chart per worker count, a line per loading mode.
for w in WORKERS:
    fig, ax = plt.subplots(figsize=(6, 4))
    for (sp, label), color in zip(MODES, MODE_COLORS):
        ax.plot(SHARE_LABELS, [med(w, sp, s) for s in SHARES], marker="o", color=color, label=label)
    ax.set_xlabel("global_queue_share_per_worker")
    ax.set_ylabel("throughput (k ops/s)")
    ax.set_title(f"{w} workers")
    ax.grid(True, alpha=0.3)
    ax.set_ylim(0, YMAX)
    ax.legend(title="loading mode")
    fig.tight_layout()
    fig.savefig(f"{OUT}/worker_{w}.svg", bbox_inches="tight")
    plt.close(fig)

print(f"wrote {OUT}/headline.svg and charts/worker_{{{','.join(map(str, WORKERS))}}}.svg")
