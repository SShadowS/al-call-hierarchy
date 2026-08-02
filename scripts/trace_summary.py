#!/usr/bin/env python3
"""Summarize an alsem Chrome-trace: SELF time first, inclusive second.

SELF time (a span's duration minus its nested children on the same tid) is the
column that ranks work. Ranking by inclusive total hid 24.8% of an 8020 run --
18.9 s inside `analyze.total` and `l4_l5.run_detectors`, two long-lived brackets
whose children do not tile them -- for this track's entire history.

Usage: scripts/trace_summary.py <trace.json> [<trace.json> ...]
"""
import json
import sys
from collections import defaultdict


def agg(path):
    """Return (inclusive_ms, self_ms, count, peak_mb) keyed by span name."""
    with open(path, "r", encoding="utf-8") as f:
        events = json.load(f)
    stack = defaultdict(list)          # tid -> [name, start_ts, child_us]
    inclusive, selft, count = defaultdict(float), defaultdict(float), defaultdict(int)
    peak = 0
    for e in events:
        tid = e.get("tid", 0)
        if e.get("ph") == "B":
            stack[tid].append([e["name"], e["ts"], 0.0])
        elif e.get("ph") == "E":
            if not stack[tid]:
                continue           # unmatched E (truncated trace) -- ignore
            name, ts0, kids = stack[tid].pop()
            dur = e["ts"] - ts0
            inclusive[name] += dur / 1000.0
            selft[name] += (dur - kids) / 1000.0
            count[name] += 1
            if stack[tid]:
                stack[tid][-1][2] += dur
            pm = (e.get("args") or {}).get("peak_mb")
            if pm and pm > peak:
                peak = pm
    return inclusive, selft, count, peak


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    for path in sys.argv[1:]:
        inclusive, selft, count, peak = agg(path)
        root = inclusive.get("analyze.total", 0.0) or 1.0
        print(f"\n=== {path}   analyze.total={root/1000:.1f}s   peak_mb={peak} ===")
        print(f"{'span':<46}{'self ms':>10}{'incl ms':>10}{'n':>6}{'self%':>8}")
        for name, ms in sorted(selft.items(), key=lambda kv: -kv[1]):
            if ms < 40:
                continue
            print(f"{name:<46}{ms:>10.1f}{inclusive[name]:>10.1f}"
                  f"{count[name]:>6}{100.0*ms/root:>7.1f}%")
    return 0


if __name__ == "__main__":
    sys.exit(main())
