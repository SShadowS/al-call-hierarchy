### ⛔ Transaction integrity — 1 medium

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: PT/D47 File Pos 1.0.0.0  —  "D47 Sender".ExportAfterModify()
  ws:src/Sender.Codeunit.al:14  Get on Rec with no SetLoadFields
  ws:src/Sender.Codeunit.al:15  accesses Rec.Name
  coverage: complete