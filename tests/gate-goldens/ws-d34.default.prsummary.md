### ⛔ Transaction integrity — 1 critical, 1 high, 1 medium, 2 info

**CRITICAL**  [d34-commit-in-loop] Commit inside a nested loop
  App: PT/D34 Commit In Loop 1.0.0.0  —  "D34 Demo".NestedCommit()
  ws:src/Codeunit.al:21  for loop
  ws:src/Codeunit.al:22  Commit (loop depth 2)
  coverage: complete

**HIGH**  [d34-commit-in-loop] Commit inside a loop
  App: PT/D34 Commit In Loop 1.0.0.0  —  "D34 Demo".DirectCommitInLoop()
  ws:src/Codeunit.al:8  for loop
  ws:src/Codeunit.al:10  Commit
  coverage: complete

**MEDIUM**  [d34-commit-in-loop] Loop reaches a Commit through a callee
  App: PT/D34 Commit In Loop 1.0.0.0  —  "D34 Demo".TransitiveCommit()
  ws:src/Codeunit.al:30  for loop
  ws:src/Codeunit.al:31  calls Persist (transitively commits)
  coverage: complete

**INFO**  [d19-unused-parameter] Procedure parameter is never used
  App: PT/D34 Commit In Loop 1.0.0.0  —  "D34 Demo".DoWork()
  ws:src/Codeunit.al:44  parameter '_n: Integer' declared but never referenced
  coverage: complete

**INFO**  [d19-unused-parameter] Procedure parameter is never used
  App: PT/D34 Commit In Loop 1.0.0.0  —  "D34 Demo".Persist()
  ws:src/Codeunit.al:46  parameter '_n: Integer' declared but never referenced
  coverage: complete