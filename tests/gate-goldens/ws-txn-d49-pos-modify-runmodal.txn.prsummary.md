### ⛔ Transaction integrity — 1 high

**HIGH**  [d49-uncommitted-write-before-ui] Uncommitted write before window-opening UI
  App: PT/D49 Pos Modify RunModal 1.0.0.0  —  "D49 Sender".ModifyThenRunModal()
  ws:src/Sender.Codeunit.al:14  DB write — transaction now dirty
  ws:src/Sender.Codeunit.al:15  UI_WINDOW_OPEN call inside open write transaction (window-opening UI)
  coverage: complete