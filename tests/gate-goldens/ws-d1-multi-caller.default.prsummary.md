### ⛔ Transaction integrity — 1 high

**HIGH**  [d1-db-op-in-loop] Database operation inside a loop
  App: PT/D1 Multi Caller 1.0.0.0  —  "D1 Multi Caller".ModifyHelper()
  ws:src/D1MultiCaller.al:26  for loop
  ws:src/D1MultiCaller.al:27  calls ModifyHelper
  ws:src/D1MultiCaller.al:19  Modify on MC Customer
  coverage: complete