# T3 arc — frozen baseline (Task 0)

- Date: 2026-07-12
- Branch: `feat/t3-lsp-migration`, worktree `.worktrees/t3`, base commit `a72f8874233811a607e1b7193488c7df2f30fb14` (master tip incl. spec+plan docs)
- CDO stats command: `./target/release/aldump.exe --program-call-graph-stats U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud | sha256sum`
- CDO output SHA-256: `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0` (byte-identical to the frozen T1-era baseline — instrument healthy)
- Full-pipeline wall-clock on CDO (release, warm dep cache, measured 2026-07-12 on master e147264-era binary): ~5.3–5.6s
- `cargo test --release -q`: exit 0, all suites green (0 failed across all binaries; doctests green)
- Env note: worktree builds require `TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al` (submodule absent in worktrees)
