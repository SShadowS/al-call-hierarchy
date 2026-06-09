### ⛔ Transaction integrity — 1 medium

**MEDIUM**  [d3-missing-setloadfields] Missing SetLoadFields before a record retrieval
  App: PT/D46 Commit In Lifecycle Pos 1.0.0.0  —  "Upgrade Handler".DoUpgrade()
  ws:src/UpgradeCU.al:14  Get on Setup with no SetLoadFields
  ws:src/UpgradeCU.al:15  accesses Setup.Description
  coverage: complete