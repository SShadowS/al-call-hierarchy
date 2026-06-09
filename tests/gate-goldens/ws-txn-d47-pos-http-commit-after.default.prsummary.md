### ⛔ Transaction integrity — 1 medium

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: PT/D47 HTTP Commit After Pos 1.0.0.0  —  "D47 Sender".SendThenCommit()
  ws:src/Sender.Codeunit.al:15  Get on Rec with no SetLoadFields
  ws:src/Sender.Codeunit.al:16  accesses Rec.Name
  coverage: complete