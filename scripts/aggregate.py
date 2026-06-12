#!/usr/bin/env python3
"""Aggregate a *.jsonl sweep file into a markdown table.

Usage:
    python3 scripts/aggregate.py <file.jsonl> <row_key> [metric] [field=value ...]

  row_key      : the field to use as table rows (e.g. "workers")
  metric       : the field to report (default "throughput_ops_per_s")
  field=value  : optional filters, e.g. "spin=0" to select one loading mode

Columns are the distinct `share` values (share=0 is shown as "legacy", the
default 1/N behaviour). Cells show the median across trials, plus the percent
change vs the legacy column.
"""
import sys
import json
import statistics
from collections import defaultdict


def main() -> None:
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)
    path, row_key = sys.argv[1], sys.argv[2]
    metric = "throughput_ops_per_s"
    filters = {}
    for a in sys.argv[3:]:
        if "=" in a:
            k, v = a.split("=", 1)
            filters[k] = v
        else:
            metric = a

    raw = defaultdict(lambda: defaultdict(list))
    for line in open(path):
        line = line.strip()
        if not line.startswith("{"):
            continue
        j = json.loads(line)
        if any(str(j.get(k)) != v for k, v in filters.items()):
            continue
        raw[j[row_key]][j["share"]].append(j[metric])

    shares = sorted({s for r in raw.values() for s in r})
    base = min(shares) if shares else 0

    def label(s):
        return "legacy" if s == 0 else f"share={s:g}"

    print("| " + row_key + " | " + " | ".join(label(s) for s in shares) + " |")
    print("|" + "---|" * (len(shares) + 1))
    for r in sorted(raw):
        med = {s: statistics.median(v) for s, v in raw[r].items() if v}
        b = med.get(base)
        cells = []
        for s in shares:
            if s not in med:
                cells.append("-")
            elif s == base or not b:
                cells.append(f"{med[s] / 1000:.0f}k")
            else:
                pct = (med[s] / b - 1) * 100
                cells.append(f"{med[s] / 1000:.0f}k ({pct:+.0f}%)")
        print(f"| {r} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
