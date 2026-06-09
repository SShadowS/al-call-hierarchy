### ⛔ Transaction integrity — 2 high

**HIGH**  [d35-commit-in-event-subscriber] Commit reachable from event subscriber (via callee)
  App: PT/D35 Commit in Subscriber 1.0.0.0  —  "D35 Demo".OnAfterValidateTransitive()
  ws:src/Codeunit.al:12  [EventSubscriber] OnAfterValidateTransitive transitively commits (mayCommit(summary) == "yes")
  coverage: complete

**HIGH**  [d35-commit-in-event-subscriber] Commit reachable from event subscriber
  App: PT/D35 Commit in Subscriber 1.0.0.0  —  "D35 Demo".OnAfterPostDirect()
  ws:src/Codeunit.al:5  [EventSubscriber] OnAfterPostDirect
  ws:src/Codeunit.al:7  Commit
  coverage: complete