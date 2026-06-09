### ⛔ Transaction integrity — 1 medium

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: PT/D49 Pos Modify RunModal 1.0.0.0  —  "D49 Sender".ModifyThenRunModal()
  ws:src/Sender.Codeunit.al:12  Get on Rec with no SetLoadFields
  ws:src/Sender.Codeunit.al:13  accesses Rec.Name
  coverage: complete