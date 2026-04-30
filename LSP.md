# LSP Integration Guide

This document describes the LSP capabilities provided by `al-call-hierarchy` and the changes required for the AL LSP wrapper (`al-language-server-python`) to integrate them.

## Server Capabilities

The server advertises the following capabilities:

```json
{
  "callHierarchyProvider": true,
  "codeLensProvider": {
    "resolveProvider": false
  },
  "textDocumentSync": {
    "openClose": true,
    "change": 0,
    "save": {
      "includeText": false
    }
  }
}
```

## Supported Methods

### Existing (Call Hierarchy)

| Method | Description |
|--------|-------------|
| `textDocument/prepareCallHierarchy` | Get call hierarchy item at position |
| `callHierarchy/incomingCalls` | Get callers of a procedure |
| `callHierarchy/outgoingCalls` | Get callees of a procedure |

### New (Code Lens)

| Method | Description |
|--------|-------------|
| `textDocument/codeLens` | Get reference counts for all procedures in a file |

### New (Diagnostics - Server Push)

| Method | Description |
|--------|-------------|
| `textDocument/publishDiagnostics` | Unused procedures and code quality warnings |

---

## AL LSP Wrapper Changes Required

### 1. Route `textDocument/codeLens` Requests

**Priority:** High
**Effort:** Low

The wrapper needs to forward Code Lens requests to this server:

```python
# In request router
if method == "textDocument/codeLens":
    return call_hierarchy_server.request(method, params)
```

Or in Go:
```go
case "textDocument/codeLens":
    return callHierarchyServer.Request(method, params)
```

### 2. Merge Diagnostics from Multiple Sources

**Priority:** Medium
**Effort:** Medium

This server now publishes diagnostics via `textDocument/publishDiagnostics`. The wrapper has two options:

#### Option A: Merge diagnostics (Recommended)

Collect diagnostics from both MS AL LSP and this server, merge by file URI, and publish combined:

```python
def on_diagnostics(source, uri, diagnostics):
    all_diagnostics[uri][source] = diagnostics
    merged = []
    for source_diags in all_diagnostics[uri].values():
        merged.extend(source_diags)
    client.publish_diagnostics(uri, merged)
```

#### Option B: Let both publish independently

If the client supports multiple diagnostic sources (most do), both servers can publish independently. Diagnostics from this server use `source: "al-call-hierarchy"`.

### 3. Advertise Combined Capabilities

**Priority:** Medium
**Effort:** Low

The wrapper should advertise the combined capabilities to the client:

```json
{
  "capabilities": {
    "callHierarchyProvider": true,
    "codeLensProvider": {
      "resolveProvider": false
    },
    "textDocumentSync": {
      "openClose": true,
      "change": 1,
      "save": { "includeText": false }
    }
  }
}
```

### 4. Forward Document Open/Close Events (Optional)

**Priority:** Low
**Effort:** Low

For future incremental diagnostics, forward document lifecycle events:

```python
if method == "textDocument/didOpen":
    call_hierarchy_server.notify(method, params)
if method == "textDocument/didClose":
    call_hierarchy_server.notify(method, params)
```

---

## Diagnostic Codes

Diagnostics published by this server:

| Code | Severity | Threshold | Description |
|------|----------|-----------|-------------|
| `unused-procedure` | Hint | 0 callers | Procedure has no callers (tagged `UNNECESSARY`) |
| `high-complexity` | Warning | ≥10 | Cyclomatic complexity exceeds critical threshold |
| `high-complexity` | Information | ≥5 | Cyclomatic complexity exceeds warning threshold |
| `too-many-parameters` | Warning | ≥7 | Parameter count exceeds critical threshold |
| `too-many-parameters` | Information | ≥4 | Parameter count exceeds warning threshold |
| `high-fan-in` | Information | >20 | Procedure has many callers |
| `long-method` | Information | >50 lines | Procedure spans many lines |

All diagnostics use `source: "al-call-hierarchy"`.

---

## Code Lens Commands

Code Lens items include a command that the client can execute:

```json
{
  "command": "al-call-hierarchy.showReferences",
  "arguments": [{
    "object": "MyCodeunit",
    "procedure": "MyProcedure",
    "uri": "file:///path/to/file.al"
  }]
}
```

The wrapper or VS Code extension should register a handler for `al-call-hierarchy.showReferences` to show the references panel.

---

## Event Subscribers in Call Hierarchy

`callHierarchy/incomingCalls` now also returns event subscribers. These appear with:

```json
{
  "from": {
    "name": "OnAfterPostSalesDoc",
    "kind": 24,  // SymbolKind.EVENT
    "detail": "MyCodeunit.OnAfterPostSalesDoc [EventSubscriber]"
  }
}
```

No wrapper changes needed - this is automatic.

---

## Summary of Wrapper Changes

| Change | Priority | Effort | Required? |
|--------|----------|--------|-----------|
| Route `textDocument/codeLens` | High | Low | Yes |
| Merge/forward diagnostics | Medium | Medium | Recommended |
| Advertise combined capabilities | Medium | Low | Recommended |
| Forward document events | Low | Low | Optional |
| Register `showReferences` command | Low | Low | Optional |
