### ⛔ Transaction integrity — 1 critical

**CRITICAL**  [d47-io-unsafe-txn] External IO inside an open write transaction
  App: PT/D47 HTTP Commit After Pos 1.0.0.0  —  "D47 Sender".SendThenCommit()
  ws:src/Sender.Codeunit.al:17  DB write — transaction now dirty
  ws:src/Sender.Codeunit.al:18  HTTP Get call inside open write transaction
  coverage: complete