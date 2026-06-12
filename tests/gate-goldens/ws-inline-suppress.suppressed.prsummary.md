### ⛔ Transaction integrity — 1 critical, 5 medium

**CRITICAL**  [d47-io-unsafe-txn] External IO inside an open write transaction
  App: Test/InlineSuppress 1.0.0.0  —  "IS Sender".WrongDirectiveIo()
  ws:src/Sender.Codeunit.al:27  DB write — transaction now dirty
  ws:src/Sender.Codeunit.al:29  HTTP Get call inside open write transaction
  coverage: complete

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: Test/InlineSuppress 1.0.0.0  —  "IS Sender".WrongDirectiveIo()
  ws:src/Sender.Codeunit.al:25  Get on Rec2 with no SetLoadFields
  ws:src/Sender.Codeunit.al:26  accesses Rec2.Name
  coverage: complete

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: Test/InlineSuppress 1.0.0.0  —  "IS Sender".UnsuppressedD3()
  ws:src/Sender.Codeunit.al:42  Get on Rec3 with no SetLoadFields
  ws:src/Sender.Codeunit.al:42  accesses Rec3.No.
  ws:src/Sender.Codeunit.al:43  accesses Rec3.Name
  coverage: complete

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: Test/InlineSuppress 1.0.0.0  —  "IS Sender".UnsuppressedD3()
  ws:src/Sender.Codeunit.al:40  FindSet on Rec3 with no SetLoadFields
  ws:src/Sender.Codeunit.al:42  accesses Rec3.No.
  ws:src/Sender.Codeunit.al:43  accesses Rec3.Name
  coverage: complete

**MEDIUM**  [d1-db-op-in-loop] Database operation inside a loop
  App: Test/InlineSuppress 1.0.0.0  —  "IS Sender".UnsuppressedD3()
  ws:src/Sender.Codeunit.al:41  repeat loop
  ws:src/Sender.Codeunit.al:42  Get on IS Rec
  coverage: complete

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: Test/InlineSuppress 1.0.0.0  —  "IS Sender".SuppressedIo()
  ws:src/Sender.Codeunit.al:11  Get on Rec with no SetLoadFields
  ws:src/Sender.Codeunit.al:12  accesses Rec.Name
  coverage: complete