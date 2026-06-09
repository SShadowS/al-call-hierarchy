### ⛔ Transaction integrity — 1 critical

**CRITICAL**  [d47-io-unsafe-txn] External IO inside an open write transaction
  App: PT/D47 File Pos 1.0.0.0  —  "D47 Sender".ExportAfterModify()
  ws:src/Sender.Codeunit.al:16  DB write — transaction now dirty
  ws:src/Sender.Codeunit.al:17  FILE call inside open write transaction
  coverage: complete