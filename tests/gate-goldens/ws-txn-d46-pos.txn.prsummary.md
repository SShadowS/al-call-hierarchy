### ⛔ Transaction integrity — 2 high

**HIGH**  [d46-commit-in-lifecycle] Commit reachable from Install/Upgrade lifecycle trigger
  App: PT/D46 Commit In Lifecycle Pos 1.0.0.0  —  "Install Handler".OnInstallAppPerCompany()
  ws:src/InstallCU.al:22  OnInstallAppPerCompany (Install trigger)
  ws:src/InstallCU.al:24  calls DoSetup
  ws:src/InstallCU.al:35  Commit
  coverage: complete

**HIGH**  [d46-commit-in-lifecycle] Commit reachable from Install/Upgrade lifecycle trigger
  App: PT/D46 Commit In Lifecycle Pos 1.0.0.0  —  "Upgrade Handler".OnUpgradePerCompany()
  ws:src/UpgradeCU.al:5  OnUpgradePerCompany (Upgrade trigger)
  ws:src/UpgradeCU.al:7  calls DoUpgrade
  ws:src/UpgradeCU.al:18  Commit
  coverage: complete