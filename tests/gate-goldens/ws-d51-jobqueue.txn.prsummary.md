### ⛔ Transaction integrity — 1 low

**LOW**  [d51-retry-side-effect-duplication] External request may be duplicated on retry
  App: PT/D51 Retry Side Effect Duplication JobQueue 1.0.0.0  —  "D51 Sender".PostThenError()
  ws:src/Sender.Codeunit.al:14  HTTP Post request — if this routine is retried the request may be re-issued
  coverage: complete