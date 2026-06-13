### ⛔ Transaction integrity — 1 high, 1 medium

**HIGH**  [d48-io-in-loop] External IO inside a loop
  App: PT/D48 IO In Loop Pos 1.0.0.0  —  "D48 Sender".SendOne()
  ws:src/D48Pos.Codeunit.al:30  repeat loop
  ws:src/D48Pos.Codeunit.al:32  calls SendOne
  ws:src/D48Pos.Codeunit.al:40  calls SendOne
  ws:src/D48Pos.Codeunit.al:15  HTTP Send
  coverage: complete

**MEDIUM**  [d48-io-in-loop] External IO inside a loop
  App: PT/D48 IO In Loop Pos 1.0.0.0  —  "D48 File Loop".ExportAll()
  ws:src/D48Pos.Codeunit.al:71  repeat loop
  ws:src/D48Pos.Codeunit.al:73  FILE
  coverage: complete