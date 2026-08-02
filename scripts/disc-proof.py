#!/usr/bin/env python3
"""Run discrimination proofs: break the code, watch the test FAIL, revert, watch it PASS.

A test that has never been seen to fail is not evidence. And a proof that comes
back GREEN-WHEN-BROKEN is evidence about the TEST, not the code -- diagnose it,
do not accept it. In the 2026-08-02 arc, 3 of 5 proofs came back green and all
three were real test defects.

Spec file: JSON list of
  {"label": str, "file": str, "test": str, "old": str, "new": str, "count": 1}

Usage: python scripts/disc-proof.py <spec.json>
Exit 0 iff every proof is GOOD (fails when broken, passes when restored).
"""
import io
import json
import subprocess
import sys


def run_test(filt):
    r = subprocess.run(
        ["cargo", "test", "-p", "al-call-hierarchy", "--lib", filt],
        capture_output=True, text=True,
    )
    return r.returncode == 0


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    specs = json.load(open(sys.argv[1], encoding="utf-8"))
    ok = True
    for s in specs:
        path, old, new = s["file"], s["old"], s["new"]
        want = s.get("count", 1)
        orig = io.open(path, encoding="utf-8").read()
        n = orig.count(old)
        if n != want:
            # An unasserted scripted break proves nothing, and its green run
            # reads exactly like a passing test. rustfmt reflowing an anchor has
            # silently produced this in the past.
            print(f"BAD   {s['label']:48} PATCH-NOT-UNIQUE (found {n}, want {want})")
            ok = False
            continue
        io.open(path, "w", encoding="utf-8", newline="\n").write(orig.replace(old, new, 1))
        broken_passes = run_test(s["test"])
        io.open(path, "w", encoding="utf-8", newline="\n").write(orig)
        restored_passes = run_test(s["test"])
        assert io.open(path, encoding="utf-8").read() == orig, f"{path} not restored!"
        good = (not broken_passes) and restored_passes
        ok = ok and good
        print(f"{'GOOD' if good else 'BAD':5} {s['label']:48} "
              f"broken={'PASS(!)' if broken_passes else 'FAIL':8} "
              f"restored={'PASS' if restored_passes else 'FAIL(!)'}")
    print("ALL PROOFS GOOD" if ok else "SOME PROOFS BAD -- diagnose the TEST, not just the code")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
