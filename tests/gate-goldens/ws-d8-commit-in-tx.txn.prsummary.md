### ⛔ Transaction integrity — 2 high

**HIGH**  [d35-commit-in-event-subscriber] Commit reachable from event subscriber
  App: PT/D8 Tx Span 1.0.0.0  —  "D8BadSubscriber".HandlePosted()
  ws:src/subscriber.al:4  [EventSubscriber] HandlePosted
  ws:src/subscriber.al:11  Commit
  coverage: complete

**HIGH**  [d8-commit-in-transaction] Commit inside a posting transaction span
  App: PT/D8 Tx Span 1.0.0.0  —  "D8BadSubscriber".HandlePosted()
  ws:src/posting.al:3  transaction-managing routine: PostSalesDoc
  ws:src/subscriber.al:11  Commit inside PostSalesDoc's transaction span
  coverage: complete