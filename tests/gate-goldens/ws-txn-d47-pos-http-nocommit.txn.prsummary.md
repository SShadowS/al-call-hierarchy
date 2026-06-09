### ⛔ Transaction integrity — 1 critical

**CRITICAL**  [d47-io-unsafe-txn] External IO inside an open write transaction
  App: PT/D47 HTTP No Commit Pos 1.0.0.0  —  "D47 Sender".SendAfterModify()
  ws:src/Sender.Codeunit.al:16  DB write — transaction now dirty
  ws:src/Sender.Codeunit.al:17  HTTP Get call inside open write transaction
  coverage: complete