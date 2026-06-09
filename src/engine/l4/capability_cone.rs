//! L4 CAPABILITY CONE + COVERAGE (R3a-3) — faithful port of al-sem's
//! `src/engine/capability-cone.ts` (`composeInheritedCones` + the supporting
//! cone-build recurrences) and the cone/coverage wire-in in
//! `src/engine/summary-runner.ts` (~505-680) + the R3a-3 stable projection
//! (`scripts/r3a3-projection.ts` `projectR3a3`).
//!
//! Three layers:
//!   1. DIRECT facts — per-routine `capabilityFactsDirect`, built from the
//!      RESOLVED L3 routine (table family from `record_operations`, dispatch
//!      family from object-run call sites) + the L4 publisher-fact injection
//!      from the resolved event graph. This mirrors al-sem's L4 `extractCapabilities`
//!      (which reads `routine.features.recordOperations` with the L3-RESOLVED
//!      `tableId`) + the publisher-fact injection in summary-runner.ts.
//!   2. The CONE — `compose_inherited_cones`: a single fused bottom-up walk over
//!      the SCC condensation of `typedEdges`. Per SCC it builds a fact cone
//!      (members' direct facts at dist 0 + each successor cone shifted +1) and a
//!      coverage cone, emits per-routine inherited facts (SHORTEST-distance
//!      witness, `inheritedFactKey` dedup, equal-distance tie-breaker, canonical
//!      sort) + the coverage `CoverageRecord` roll-up, then refcount-frees
//!      downstream cones.
//!   3. The PROJECTION — `project_r3a3`: the stable-id comparison surface the
//!      R3a-3 vectors / goldens carry.
//!
//! All ids are INTERNAL until the projection. No `HashMap` iteration reaches the
//! output: the cone fact maps are `BTreeMap` (key-ordered) and every output Vec
//! is explicitly sorted. The walk terminates on cyclic/deep graphs because it
//! runs over the ACYCLIC SCC condensation; within a recursive SCC the BFS visits
//! each member at most once.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::combined_graph::{build_combined_graph, CombinedGraph, TypedEdge};
use super::scc::{tarjan_scc, Scc, SccInputGraph, SccResult};
use crate::engine::ids::to_stable_object_id;
use crate::engine::l2::features::{PCallSite, PCallee, PExpressionInfo, POperationSite};
use crate::engine::l3::call_resolver::{resolve_calls, DeclaredDependency};
use crate::engine::l3::event_graph::{build_event_graph, EventGraph, EventSymbol};
use crate::engine::l3::l3_workspace::{L3Resolved, L3Routine, L3Workspace};
use crate::engine::l3::symbol_table::SymbolTable;

// ===========================================================================
// Internal CapabilityFact (FULL form — internal ids). Mirrors al-sem
// `model/capability.ts` `CapabilityFact`, dropping `subject` (the projection
// key) and carrying the L3-resolved `resource_id`.
// ===========================================================================

/// A `ValueSource` (internal form — table-field tableId is INTERNAL until
/// projected). Mirrors al-sem `ValueSource`.
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub enum ValueSource {
    Literal {
        value: String,
    },
    Enum {
        enum_name: String,
        member: Option<String>,
    },
    ConstantVar {
        var_name: String,
        initializer: Box<ValueSource>,
    },
    Parameter {
        index: u32,
        var_name: String,
    },
    TableField {
        table_id: String,
        field_name: String,
    },
    Expression,
    Unknown,
}

/// Per-resourceKind extra semantics (internal form). Only the variants the
/// vector-exercised families produce are modelled with structure; the rest pass
/// through as opaque JSON for projection (so the full corpus in Task 3 can be
/// extended without reshaping the cone).
#[derive(Debug, Clone, PartialEq, salsa::Update)]
pub enum CapabilityExtra {
    Table {
        record_variable_id: Option<String>,
        temp_state: Option<crate::engine::l2::features::PTempState>,
        op_subtype: Option<String>,
    },
    Dispatch {
        object_type: String,
        modal: Option<bool>,
    },
    Event {
        event_class: String,
        include_sender: Option<bool>,
    },
    /// HttpExtra (al-sem `model/capability.ts`). `method` + optional `bodyArgSource`.
    Http {
        method: String,
        body_arg_source: Option<ValueSource>,
    },
    /// StorageExtra (al-sem `model/capability.ts`). Optional key/value sources +
    /// scope (`User` | `Company` | `Module` | `unknown`).
    Storage {
        key_arg_source: Option<ValueSource>,
        value_arg_source: Option<ValueSource>,
        scope: Option<String>,
    },
}

/// One normalized direct/inherited capability fact (internal form). `subject` is
/// kept for `repKey` tie-break parity but excluded from the projection.
#[derive(Debug, Clone, PartialEq, salsa::Update)]
pub struct CapabilityFact {
    pub subject: String,
    pub op: String,
    pub resource_kind: String,
    /// Internal resourceId (TableId / EventId / ObjectId) — projected to stable.
    pub resource_id: Option<String>,
    pub resource_arg_source: Option<ValueSource>,
    pub confidence: String,
    pub provenance: String,
    pub via: String,
    pub witness_operation_id: Option<String>,
    pub witness_callsite_id: Option<String>,
    pub extra: Option<CapabilityExtra>,
}

/// One per-routine coverage record (internal form). Mirrors al-sem
/// `CoverageRecord`.
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub struct CoverageRecord {
    pub subject: String,
    pub direct_status: String,
    pub inherited_status: String,
    pub reasons: Vec<String>,
    pub unknown_targets: Vec<String>,
}

// ===========================================================================
// DIRECT facts — per-routine `capabilityFactsDirect`, built over the RESOLVED
// L3 routine + the publisher-fact injection. Mirrors al-sem's L4
// `extractCapabilities` read of the RESOLVED features + summary-runner.ts
// publisher injection.
// ===========================================================================

/// Map an AL `RecordOpType` to a `CapabilityOp`, or `None` for state-only /
/// filter ops (SetRange, SetFilter, Init, ...). Mirrors al-sem `table.ts`
/// `mapOp`.
fn map_table_op(op: &str) -> Option<&'static str> {
    match op {
        "Get" | "Find" | "FindFirst" | "FindLast" | "FindSet" | "IsEmpty" | "Count"
        | "CountApprox" | "Next" | "CalcFields" | "CalcSums" | "TestField" => Some("read"),
        "Modify" | "ModifyAll" | "Validate" | "Copy" | "TransferFields" => Some("modify"),
        "Insert" => Some("insert"),
        "Delete" | "DeleteAll" => Some("delete"),
        _ => None,
    }
}

/// Map an object-run kind to the dispatch resourceKind (al-sem `dispatch.ts`).
fn object_type_to_resource_kind(object_type: &str) -> &'static str {
    match object_type {
        "Codeunit" => "codeunit",
        "Page" => "page",
        "Report" => "report",
        _ => "codeunit",
    }
}

/// Map an EventSymbol.eventKind to its CapabilityExtra event class. Faithful port
/// of al-sem `summary-runner.ts` `mapEventKindToClass`: business→Business,
/// internal→Internal, trigger→Trigger, EVERYTHING else (incl. "integration" /
/// "unknown") → Integration. The Rust event graph emits lowercase kinds
/// ("integration" / "business" / "unknown").
fn map_event_kind_to_class(event_kind: &str) -> &'static str {
    match event_kind {
        "business" => "Business",
        "internal" => "Internal",
        "trigger" => "Trigger",
        _ => "Integration",
    }
}

/// Map an HttpClient method name to its `HttpExtra.method` literal, or `None`
/// when the method isn't an HTTP verb (al-sem `http.ts` HTTP_METHOD_SET).
fn http_method(method: &str) -> Option<&'static str> {
    match method {
        "Send" => Some("Send"),
        "Get" => Some("Get"),
        "Post" => Some("Post"),
        "Put" => Some("Put"),
        "Delete" => Some("Delete"),
        "Patch" => Some("Patch"),
        _ => None,
    }
}

/// Map a lowercased IsolatedStorage method to its capability op (al-sem
/// `isolated-storage.ts` ISOLATED_STORAGE_OPS), or `None`.
fn isolated_storage_op(method_lc: &str) -> Option<&'static str> {
    match method_lc {
        "get" | "getencrypted" | "contains" => Some("store-read"),
        "set" | "setencrypted" => Some("store-write"),
        "delete" => Some("store-delete"),
        _ => None,
    }
}

/// Parse a DataScope enum text → `StorageExtra.scope` (al-sem `parseDataScope`).
fn parse_data_scope(text: &str) -> String {
    let lower = text.to_lowercase();
    if lower.contains("::company") {
        "Company".to_string()
    } else if lower.contains("::user") {
        "User".to_string()
    } else if lower.contains("::module") {
        "Module".to_string()
    } else {
        "unknown".to_string()
    }
}

/// True when a declared type names a TempBlob (al-sem `isTempBlobType`).
fn is_temp_blob_type(t: &str) -> bool {
    let lc = t.to_lowercase();
    lc.contains("temp blob") || lc == "tempblob"
}

/// True when a declared type names a Page or Report (al-sem
/// `ui-window-open.ts` `isPageOrReportType`).
fn is_page_or_report_type(declared_type: &str) -> bool {
    let lower = declared_type.to_lowercase();
    lower == "page"
        || lower == "report"
        || lower.starts_with("page ")
        || lower.starts_with("report ")
}

/// Confidence derivation from a `ValueSource` (al-sem `confidenceFromSource`).
fn confidence_from_source(vs: &ValueSource) -> &'static str {
    match vs {
        ValueSource::Literal { .. } | ValueSource::Enum { .. } => "static",
        ValueSource::ConstantVar { initializer, .. } => confidence_from_source(initializer),
        ValueSource::Parameter { .. } => "userDynamic",
        ValueSource::TableField { .. } => "configDynamic",
        ValueSource::Expression | ValueSource::Unknown => "unresolved",
    }
}

/// A minimal variable index for value-source classification: lowercased name →
/// (is_parameter, parameter_index, declared_type, table_id?). Built from the L3
/// routine's record vars + lexical variables.
struct VarInfo {
    is_parameter: bool,
    parameter_index: u32,
    /// Declared type — `receiverTypeOf` (http/file-blob/ui-window-open receiver
    /// classification) + the member-expression (table-field) record check read it.
    declared_type: String,
    /// Resolved internal TableId when this var is a record variable — feeds the
    /// member-expression (table-field) value-source branch (`classifyMemberExpression`).
    table_id: Option<String>,
    /// The L2-captured one-hop initializer as a `ValueSource` (al-sem
    /// `VariableSymbol.initializer`). Feeds `classifyIdentifier`'s constant-var
    /// resolution + the one-hop var-to-var alias chase.
    initializer: Option<ValueSource>,
}

/// Classify a call-argument `PExpressionInfo` into a `ValueSource`. A focused
/// port of al-sem `value-source.ts` covering the literal / enum / database-ref /
/// parameter / member-expression (table-field) forms the dispatch + background +
/// IO families need. Constant-var initializer chasing requires the L2 captured
/// initializer (dropped at L3); unresolved local identifiers degrade to
/// `Expression` (matching al-sem when no initializer is captured).
fn classify_value_source(
    info: Option<&PExpressionInfo>,
    variables: &HashMap<String, VarInfo>,
) -> ValueSource {
    let Some(info) = info else {
        return ValueSource::Unknown;
    };
    match info.kind.as_str() {
        "string_literal" => ValueSource::Literal {
            value: info
                .value
                .clone()
                .unwrap_or_else(|| strip_single_quotes(&info.text)),
        },
        "integer" | "decimal" | "boolean" => ValueSource::Literal {
            value: info
                .value
                .clone()
                .unwrap_or_else(|| info.text.trim().to_string()),
        },
        "qualified_enum_value" | "database_reference" => {
            let enum_name = info
                .qualifier
                .as_deref()
                .map(strip_double_quotes)
                .map(|s| s.to_string());
            // al-sem value-source.ts: `info.member ?? info.value ?? ""` — the `?? ""`
            // floor means `member` is ALWAYS a string (worst case ""), so it is always
            // serialized. Without the floor, a qualified_enum_value/database_reference
            // whose ExpressionInfo carries neither member nor value would omit the key
            // where al-sem emits "". (Corpus-invisible: corpus enum refs are well-formed.)
            let member = info
                .member
                .clone()
                .or_else(|| info.value.clone())
                .or(Some(String::new()));
            ValueSource::Enum {
                enum_name: enum_name.unwrap_or_default(),
                member,
            }
        }
        "identifier" | "quoted_identifier" => {
            let name = info
                .value
                .clone()
                .unwrap_or_else(|| info.text.clone())
                .to_lowercase();
            classify_identifier(&name, variables, 0)
        }
        "member_expression" => classify_member_expression(&info.text, variables),
        "unary_expression" => match &info.value {
            Some(v) => ValueSource::Literal { value: v.clone() },
            None => ValueSource::Expression,
        },
        _ => ValueSource::Expression,
    }
}

const MAX_CHASE_DEPTH: u32 = 3;

/// Resolve an identifier name to a `ValueSource`. Faithful port of al-sem
/// `value-source.ts` `classifyIdentifier`: a parameter → `parameter`; a local
/// with a resolved initializer → that initializer (one-hop var-to-var alias
/// chase, capped at `MAX_CHASE_DEPTH`); a local with no / opaque initializer →
/// `constant-var`. Unknown name → `expression`.
fn classify_identifier(
    name_lower: &str,
    variables: &HashMap<String, VarInfo>,
    depth: u32,
) -> ValueSource {
    let Some(sym) = variables.get(name_lower) else {
        return ValueSource::Expression;
    };
    if sym.is_parameter {
        return ValueSource::Parameter {
            index: sym.parameter_index,
            var_name: name_lower.to_string(),
        };
    }
    let init = sym.initializer.clone();
    match &init {
        None | Some(ValueSource::Unknown) | Some(ValueSource::Expression) => {
            // No initializer captured or it's already opaque — emit constant-var.
            ValueSource::ConstantVar {
                var_name: name_lower.to_string(),
                initializer: Box::new(init.unwrap_or(ValueSource::Unknown)),
            }
        }
        Some(init_vs) => {
            if depth >= MAX_CHASE_DEPTH {
                // Depth cap — keep the raw initializer, don't recurse.
                return ValueSource::ConstantVar {
                    var_name: name_lower.to_string(),
                    initializer: Box::new(init_vs.clone()),
                };
            }
            if let ValueSource::ConstantVar {
                var_name: inner, ..
            } = init_vs
            {
                let deeper = classify_identifier(inner, variables, depth + 1);
                if matches!(
                    deeper,
                    ValueSource::Literal { .. }
                        | ValueSource::Enum { .. }
                        | ValueSource::Parameter { .. }
                ) {
                    return deeper;
                }
                return ValueSource::ConstantVar {
                    var_name: name_lower.to_string(),
                    initializer: Box::new(deeper),
                };
            }
            // Initializer already a resolved kind (literal / enum / parameter / table-field).
            init_vs.clone()
        }
    }
}

/// Classify a `member_expression` text (`Receiver.Field`) as a `table-field`
/// ValueSource when the receiver resolves to a record-typed variable; else
/// `expression`. Faithful port of al-sem `value-source.ts`
/// `classifyMemberExpression`. The first `.` separates receiver from field.
fn classify_member_expression(text: &str, variables: &HashMap<String, VarInfo>) -> ValueSource {
    let Some(dot_idx) = text.find('.') else {
        return ValueSource::Expression;
    };
    let receiver_raw = text[..dot_idx].trim();
    let field_raw = text[dot_idx + 1..].trim();
    let receiver_lower = receiver_raw.to_lowercase();
    let Some(sym) = variables.get(&receiver_lower) else {
        return ValueSource::Expression;
    };
    let decl = sym.declared_type.to_lowercase();
    let is_record =
        decl.starts_with("record ") || decl == "record" || decl.starts_with("recordref");
    if !is_record {
        return ValueSource::Expression;
    }
    let field_name = strip_double_quotes(field_raw).to_string();
    let table_id = sym
        .table_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    ValueSource::TableField {
        table_id,
        field_name,
    }
}

/// Parse an L2-captured `VariableSymbol.initializer` JSON value into a
/// `ValueSource`. Mirrors al-sem's `ValueSource` JSON shape. Unknown / malformed
/// shapes degrade to `ValueSource::Unknown` (engine-never-throws).
fn value_source_from_json(v: &serde_json::Value) -> ValueSource {
    let Some(kind) = v.get("kind").and_then(|k| k.as_str()) else {
        return ValueSource::Unknown;
    };
    match kind {
        "literal" => ValueSource::Literal {
            value: v
                .get("value")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        "enum" => ValueSource::Enum {
            enum_name: v
                .get("enumName")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            member: v
                .get("member")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        },
        "constant-var" => ValueSource::ConstantVar {
            var_name: v
                .get("varName")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            initializer: Box::new(
                v.get("initializer")
                    .map(value_source_from_json)
                    .unwrap_or(ValueSource::Unknown),
            ),
        },
        "parameter" => ValueSource::Parameter {
            index: v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            var_name: v
                .get("varName")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        "table-field" => ValueSource::TableField {
            table_id: v
                .get("tableId")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string(),
            field_name: v
                .get("fieldName")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
        },
        "expression" => ValueSource::Expression,
        _ => ValueSource::Unknown,
    }
}

fn strip_single_quotes(s: &str) -> String {
    let t = s.trim();
    let b = t.as_bytes();
    if b.len() >= 2 && b[0] == b'\'' && b[b.len() - 1] == b'\'' {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn strip_double_quotes(s: &str) -> &str {
    let t = s.trim();
    let b = t.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

/// Build the per-routine direct capability facts in al-sem extractor order
/// (table → dispatch → … → events) for the families the source-only cone needs,
/// PLUS the L4 publisher-fact injection. Returns `(facts, status, reasons)`.
///
/// Opaque / parse-incomplete routines yield `([], "unknown", reasons)` mirroring
/// the summary-runner.ts:553-559 override.
pub(crate) fn direct_facts_for_routine(
    routine: &L3Routine,
    publisher_events: &[&EventSymbol],
) -> (Vec<CapabilityFact>, String, Vec<String>) {
    if !routine.body_available {
        return (
            Vec::new(),
            "unknown".to_string(),
            vec!["opaque-dependency".to_string()],
        );
    }
    if routine.parse_incomplete {
        return (
            Vec::new(),
            "unknown".to_string(),
            vec!["parse-incomplete".to_string()],
        );
    }

    // Variable index for value-source classification + receiverTypeOf. Built from
    // the L3 lexical variables ALONE (which already include the parameters as
    // VariableSymbols with isParameter=true), LAST-wins per lowercased name —
    // mirroring al-sem's orchestrator `for (v of features.variables)
    // variables.set(v.name, v)`. Record-var TableIds are folded in afterwards so
    // the member-expression (table-field) value-source branch can resolve.
    //
    // NOTE: al-sem reads `sym.tableId` off the VariableSymbol, which is NOT
    // populated for source-only routines (the VariableSymbol carries no resolved
    // tableId — only the RecordVariable does), so a `Receiver.Field` member-expr
    // resolves to `table-field` with `tableId: "unknown"`. We therefore DO NOT
    // fold the resolved record-var tableId here: doing so would over-resolve vs
    // al-sem (confirmed by the ws-policy-api-dynamic-dispatch golden, which carries
    // `tableId: "unknown"`).
    let mut variables: HashMap<String, VarInfo> = HashMap::new();
    for v in &routine.variables {
        variables.insert(
            v.name.to_lowercase(),
            VarInfo {
                is_parameter: v.is_parameter,
                parameter_index: v.parameter_index.unwrap_or(0),
                declared_type: v.declared_type.clone(),
                table_id: None,
                initializer: v.initializer.as_ref().map(value_source_from_json),
            },
        );
    }

    // ── Unreachable exclusion (extractor.ts:100-142) ───────────────────────
    // Operation sites / call sites with controlContext === "unreachable" never
    // produce capability facts. recordOperations share IDs with operationSites,
    // so exclude record ops whose id is in the unreachable-op-id set.
    let mut unreachable_op_ids: BTreeSet<String> = BTreeSet::new();
    for op in &routine.operation_sites {
        if op.control_context.as_deref() == Some("unreachable") {
            unreachable_op_ids.insert(op.id.clone());
        }
    }
    let record_ops: Vec<&crate::engine::l3::l3_workspace::L3RecordOperation> = routine
        .record_operations
        .iter()
        .filter(|op| !unreachable_op_ids.contains(&op.id))
        .collect();
    let call_sites: Vec<&PCallSite> = routine
        .call_sites
        .iter()
        .filter(|cs| cs.control_context.as_deref() != Some("unreachable"))
        .collect();
    let operation_sites: Vec<&POperationSite> = routine
        .operation_sites
        .iter()
        .filter(|op| op.control_context.as_deref() != Some("unreachable"))
        .collect();

    let mut facts: Vec<CapabilityFact> = Vec::new();
    let reasons: Vec<String> = Vec::new();

    // `receiverTypeOf` — declared type of a named receiver, else "unknown"
    // (extractor.ts:147). Used by http / file-blob / ui-window-open families.
    let receiver_type_of = |name: &str| -> String {
        variables
            .get(&name.to_lowercase())
            .map(|v| v.declared_type.clone())
            .unwrap_or_else(|| "unknown".to_string())
    };

    // ── table family (al-sem table.ts) ─────────────────────────────────────
    for op in &record_ops {
        let Some(cap_op) = map_table_op(&op.op) else {
            continue;
        };
        let resource_id = op.table_id.clone();
        let confidence = if resource_id.is_some() {
            "static"
        } else {
            "unresolved"
        };
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: cap_op.to_string(),
            resource_kind: "table".to_string(),
            resource_id,
            resource_arg_source: None,
            confidence: confidence.to_string(),
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: Some(op.id.clone()),
            witness_callsite_id: None,
            extra: Some(CapabilityExtra::Table {
                record_variable_id: op.record_variable_id.clone(),
                temp_state: op.temp_state.clone(),
                op_subtype: Some(op.op.clone()),
            }),
        });
    }

    // ── commit family (al-sem commit.ts) ───────────────────────────────────
    // One fact per operationSite with kind === "commit".
    for op in &operation_sites {
        if op.kind == "commit" {
            facts.push(CapabilityFact {
                subject: routine.id.clone(),
                op: "commit".to_string(),
                resource_kind: "transaction".to_string(),
                resource_id: None,
                resource_arg_source: None,
                confidence: "static".to_string(),
                provenance: "direct".to_string(),
                via: "self".to_string(),
                witness_operation_id: Some(op.id.clone()),
                witness_callsite_id: None,
                extra: None,
            });
        }
    }

    // ── dispatch family (al-sem dispatch.ts) ───────────────────────────────
    for cs in &call_sites {
        match &cs.callee {
            PCallee::ObjectRun { object_kind, .. } => {
                let object_type = object_kind.clone();
                let target = classify_value_source(cs.argument_infos.first(), &variables);
                let confidence = confidence_from_source(&target).to_string();
                facts.push(CapabilityFact {
                    subject: routine.id.clone(),
                    op: "execute".to_string(),
                    resource_kind: object_type_to_resource_kind(&object_type).to_string(),
                    resource_id: None,
                    resource_arg_source: Some(target),
                    confidence,
                    provenance: "direct".to_string(),
                    via: "self".to_string(),
                    witness_operation_id: None,
                    witness_callsite_id: Some(cs.id.clone()),
                    extra: Some(CapabilityExtra::Dispatch {
                        object_type,
                        modal: None,
                    }),
                });
            }
            PCallee::Member { receiver, method } => {
                let key = format!("{}|{}", receiver.to_lowercase(), method.to_lowercase());
                let (object_type, modal): (&str, Option<bool>) = match key.as_str() {
                    "page|runmodal" => ("Page", Some(true)),
                    "report|execute" => ("Report", None),
                    _ => continue,
                };
                let target = classify_value_source(cs.argument_infos.first(), &variables);
                let confidence = confidence_from_source(&target).to_string();
                facts.push(CapabilityFact {
                    subject: routine.id.clone(),
                    op: "execute".to_string(),
                    resource_kind: object_type_to_resource_kind(object_type).to_string(),
                    resource_id: None,
                    resource_arg_source: Some(target),
                    confidence,
                    provenance: "direct".to_string(),
                    via: "self".to_string(),
                    witness_operation_id: None,
                    witness_callsite_id: Some(cs.id.clone()),
                    extra: Some(CapabilityExtra::Dispatch {
                        object_type: object_type.to_string(),
                        modal,
                    }),
                });
            }
            _ => {}
        }
    }

    // ── http family (al-sem http.ts) ───────────────────────────────────────
    // HttpClient member calls. receiverTypeOf(receiver) === "HttpClient".
    for cs in &call_sites {
        let PCallee::Member { receiver, method } = &cs.callee else {
            continue;
        };
        if receiver_type_of(receiver) != "HttpClient" {
            continue;
        }
        let Some(http_method) = http_method(method) else {
            continue;
        };
        let is_send = http_method == "Send";
        // .Send(Request, Response): arg[0] is the body, no URL.
        // .Post/.Put/.Patch(Url, Request, Response): arg[0] URL, arg[1] body.
        let (url_info, body_info) = if is_send {
            (None, cs.argument_infos.first())
        } else {
            (cs.argument_infos.first(), cs.argument_infos.get(1))
        };
        let url_source = match url_info {
            Some(i) => classify_value_source(Some(i), &variables),
            None => ValueSource::Unknown,
        };
        let body_arg_source = body_info.map(|i| classify_value_source(Some(i), &variables));
        let confidence = confidence_from_source(&url_source).to_string();
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "send".to_string(),
            resource_kind: "http".to_string(),
            resource_id: None,
            resource_arg_source: Some(url_source),
            confidence,
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: Some(CapabilityExtra::Http {
                method: http_method.to_string(),
                body_arg_source,
            }),
        });
    }

    // ── telemetry family (al-sem telemetry.ts) ─────────────────────────────
    // Session.LogMessage(...) member OR bare LogMessage(...).
    for cs in &call_sites {
        let matches = match &cs.callee {
            PCallee::Member { receiver, method } => {
                receiver.to_lowercase() == "session" && method.to_lowercase() == "logmessage"
            }
            PCallee::Bare { name } => name.to_lowercase() == "logmessage",
            _ => false,
        };
        if !matches {
            continue;
        }
        let event_id_source = match cs.argument_infos.first() {
            Some(i) => classify_value_source(Some(i), &variables),
            None => ValueSource::Unknown,
        };
        let confidence = confidence_from_source(&event_id_source).to_string();
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "log".to_string(),
            resource_kind: "telemetry".to_string(),
            resource_id: None,
            resource_arg_source: Some(event_id_source),
            confidence,
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: None,
        });
    }

    // ── isolated-storage family (al-sem isolated-storage.ts) ───────────────
    for cs in &call_sites {
        let PCallee::Member { receiver, method } = &cs.callee else {
            continue;
        };
        if receiver.to_lowercase() != "isolatedstorage" {
            continue;
        }
        let Some(op) = isolated_storage_op(&method.to_lowercase()) else {
            continue;
        };
        let key_source = match cs.argument_infos.first() {
            Some(i) => classify_value_source(Some(i), &variables),
            None => ValueSource::Unknown,
        };
        let confidence = confidence_from_source(&key_source).to_string();
        // store-write: capture value arg (arg[1]) + scope (arg[2]).
        let (value_arg_source, scope) = if op == "store-write" {
            let value = cs
                .argument_infos
                .get(1)
                .map(|i| classify_value_source(Some(i), &variables));
            let scope = cs.argument_infos.get(2).map(|i| parse_data_scope(&i.text));
            (value, scope)
        } else {
            (None, None)
        };
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: op.to_string(),
            resource_kind: "isolated-storage".to_string(),
            resource_id: None,
            resource_arg_source: Some(key_source.clone()),
            confidence,
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: Some(CapabilityExtra::Storage {
                key_arg_source: Some(key_source),
                value_arg_source,
                scope,
            }),
        });
    }

    // ── hyperlink family (al-sem hyperlink.ts) ─────────────────────────────
    // Bare Hyperlink(url) → op=open, resourceKind=ui.
    for cs in &call_sites {
        let PCallee::Bare { name } = &cs.callee else {
            continue;
        };
        if name.to_lowercase() != "hyperlink" {
            continue;
        }
        let url_source = match cs.argument_infos.first() {
            Some(i) => classify_value_source(Some(i), &variables),
            None => ValueSource::Unknown,
        };
        let confidence = confidence_from_source(&url_source).to_string();
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "open".to_string(),
            resource_kind: "ui".to_string(),
            resource_id: None,
            resource_arg_source: Some(url_source),
            confidence,
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: None,
        });
    }

    // ── file-blob family (al-sem file-blob.ts) ─────────────────────────────
    // Write-side File / TempBlob member methods. op=write-blob, resourceKind=file.
    for cs in &call_sites {
        let PCallee::Member { receiver, method } = &cs.callee else {
            continue;
        };
        let method_lc = method.to_lowercase();
        let receiver_type = receiver_type_of(receiver);
        let is_file = receiver_type == "File"
            && matches!(method_lc.as_str(), "create" | "writealltext" | "copy");
        let is_temp_blob = is_temp_blob_type(&receiver_type) && method_lc == "createoutstream";
        if !is_file && !is_temp_blob {
            continue;
        }
        let arg_source = match cs.argument_infos.first() {
            Some(i) => classify_value_source(Some(i), &variables),
            None => ValueSource::Unknown,
        };
        let confidence = confidence_from_source(&arg_source).to_string();
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "write-blob".to_string(),
            resource_kind: "file".to_string(),
            resource_id: None,
            resource_arg_source: Some(arg_source),
            confidence,
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: None,
        });
    }

    // ── background family (al-sem background.ts) ───────────────────────────
    // TaskScheduler.CreateTask(arg0) / Session.StartSession(_, arg1) / bare
    // StartSession(_, arg1). op=start, resourceKind=background.
    for cs in &call_sites {
        let codeunit_arg_idx: Option<usize> = match &cs.callee {
            PCallee::Member { receiver, method } => {
                let r = receiver.to_lowercase();
                let m = method.to_lowercase();
                if r == "taskscheduler" && m == "createtask" {
                    Some(0)
                } else if r == "session" && m == "startsession" {
                    Some(1)
                } else {
                    None
                }
            }
            PCallee::Bare { name } => {
                if name.to_lowercase() == "startsession" {
                    Some(1)
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(idx) = codeunit_arg_idx else {
            continue;
        };
        let codeunit_source = match cs.argument_infos.get(idx) {
            Some(i) => classify_value_source(Some(i), &variables),
            None => ValueSource::Unknown,
        };
        let confidence = confidence_from_source(&codeunit_source).to_string();
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "start".to_string(),
            resource_kind: "background".to_string(),
            resource_id: None,
            resource_arg_source: Some(codeunit_source),
            confidence,
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: None,
        });
    }

    // ── ui family (al-sem ui.ts) ───────────────────────────────────────────
    // Bare Confirm / Message / Error → ui-confirm / ui-message / ui-error.
    for cs in &call_sites {
        let PCallee::Bare { name } = &cs.callee else {
            continue;
        };
        let op = match name.to_lowercase().as_str() {
            "confirm" => "ui-confirm",
            "message" => "ui-message",
            "error" => "ui-error",
            _ => continue,
        };
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: op.to_string(),
            resource_kind: "ui".to_string(),
            resource_id: None,
            resource_arg_source: None,
            confidence: "static".to_string(),
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: None,
        });
    }

    // ── ui-window-open family (al-sem ui-window-open.ts) ───────────────────
    // Bare StrMenu; Page.Run / Report.Run (object-run); Page/Report.RunModal
    // (static keyword receiver OR a Page/Report-typed variable receiver).
    for cs in &call_sites {
        let matched = match &cs.callee {
            PCallee::Bare { name } => name.to_lowercase() == "strmenu",
            PCallee::ObjectRun { object_kind, .. } => {
                object_kind == "Page" || object_kind == "Report"
            }
            PCallee::Member { receiver, method } => {
                if method.to_lowercase() != "runmodal" {
                    false
                } else {
                    let r = receiver.to_lowercase();
                    if r == "page" || r == "report" {
                        true
                    } else {
                        let declared = receiver_type_of(&r);
                        declared != "unknown" && is_page_or_report_type(&declared)
                    }
                }
            }
            _ => false,
        };
        if !matched {
            continue;
        }
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "ui-window-open".to_string(),
            resource_kind: "ui".to_string(),
            resource_id: None,
            resource_arg_source: None,
            confidence: "static".to_string(),
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: Some(cs.id.clone()),
            extra: None,
        });
    }

    // ── events family — SUBSCRIBE side (al-sem events.ts) ──────────────────
    // A routine with an [EventSubscriber(...)] attribute emits one subscribe fact.
    if routine
        .attributes_parsed
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case("EventSubscriber"))
    {
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "subscribe".to_string(),
            resource_kind: "event".to_string(),
            resource_id: None,
            resource_arg_source: None,
            confidence: "static".to_string(),
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: None,
            extra: Some(CapabilityExtra::Event {
                event_class: "Integration".to_string(),
                include_sender: None,
            }),
        });
    }

    // ── error family (al-sem error.ts) ─────────────────────────────────────
    // One fact per operationSite with kind === "error-call".
    for op in &operation_sites {
        if op.kind == "error-call" {
            facts.push(CapabilityFact {
                subject: routine.id.clone(),
                op: "error-throw".to_string(),
                resource_kind: "error".to_string(),
                resource_id: None,
                resource_arg_source: None,
                confidence: "static".to_string(),
                provenance: "direct".to_string(),
                via: "self".to_string(),
                witness_operation_id: Some(op.id.clone()),
                witness_callsite_id: None,
                extra: None,
            });
        }
    }

    // ── publisher-fact injection (summary-runner.ts:615-632) ───────────────
    // One direct `publish` fact per EventSymbol whose publisherRoutineId is this
    // routine. Appended AFTER the family facts (al-sem `facts = [...facts, f]`).
    for evt in publisher_events {
        facts.push(CapabilityFact {
            subject: routine.id.clone(),
            op: "publish".to_string(),
            resource_kind: "event".to_string(),
            resource_id: Some(evt.id.clone()),
            resource_arg_source: None,
            confidence: "static".to_string(),
            provenance: "direct".to_string(),
            via: "self".to_string(),
            witness_operation_id: None,
            witness_callsite_id: None,
            extra: Some(CapabilityExtra::Event {
                event_class: map_event_kind_to_class(&evt.event_kind).to_string(),
                include_sender: None,
            }),
        });
    }

    let status = if reasons.is_empty() {
        "complete"
    } else {
        "partial"
    };
    (facts, status.to_string(), reasons)
}

// ===========================================================================
// The CONE — ports capability-cone.ts. Internal ids throughout.
// ===========================================================================

/// A resolved outgoing typed edge (only edges with a `to`). Mirrors al-sem
/// `TypedOutEdge`.
#[derive(Debug, Clone)]
struct TypedOutEdge {
    to: String,
    kind: String,
    callsite: Option<String>,
    event_id: Option<String>,
}

/// The typed-edge graph: per-routine sorted out-edges + the unresolved sources.
/// Mirrors al-sem `TypedEdgeGraph`.
struct TypedEdgeGraph {
    nodes: Vec<String>,
    outgoing: HashMap<String, Vec<TypedOutEdge>>,
    unresolved_sources: BTreeSet<String>,
}

/// `edgeSortKey(e)` = `${kind}|${callsite ?? ""}|${eventId ?? ""}|${to}`.
fn edge_sort_key(e: &TypedOutEdge) -> String {
    format!(
        "{}|{}|{}|{}",
        e.kind,
        e.callsite.as_deref().unwrap_or(""),
        e.event_id.as_deref().unwrap_or(""),
        e.to
    )
}

/// Map a typed-edge kind to the inherited `via`. Mirrors
/// `capabilityViaForEdgeKind`.
fn capability_via_for_edge_kind(kind: &str) -> &'static str {
    match kind {
        "direct-call" | "variable-typed-call" | "interface-dispatch" => "call",
        "object-run-resolved" | "object-run-unresolved" => "object-run",
        "event-dispatch" => "event-dispatch",
        "implicit-trigger" => "implicit-trigger",
        "dependency-export" => "dependency",
        _ => "call",
    }
}

/// The callsiteId carried by a typed out-edge (al-sem `callsiteIdForEdge`).
fn callsite_id_for_edge(e: &TypedEdge) -> Option<String> {
    match e.kind.as_str() {
        "direct-call"
        | "variable-typed-call"
        | "interface-dispatch"
        | "object-run-resolved"
        | "object-run-unresolved"
        | "dependency-export" => e.callsite_id.clone(),
        _ => None,
    }
}

/// Build the typed-edge graph from the combined graph's typed edges + the node
/// list. Mirrors al-sem `buildTypedEdgeGraph`.
fn build_typed_edge_graph(graph: &CombinedGraph, nodes: &[String]) -> TypedEdgeGraph {
    let mut outgoing: HashMap<String, Vec<TypedOutEdge>> = HashMap::new();
    let mut unresolved_sources: BTreeSet<String> = BTreeSet::new();

    for edge in &graph.typed_edges {
        if edge.kind == "object-run-unresolved" {
            unresolved_sources.insert(edge.from.clone());
            continue;
        }
        let Some(to) = &edge.to else {
            continue;
        };
        let out = TypedOutEdge {
            to: to.clone(),
            kind: edge.kind.clone(),
            callsite: callsite_id_for_edge(edge),
            event_id: edge.event_id.clone(),
        };
        outgoing.entry(edge.from.clone()).or_default().push(out);
    }
    for list in outgoing.values_mut() {
        list.sort_by_key(edge_sort_key);
    }

    let mut sorted_nodes = nodes.to_vec();
    sorted_nodes.sort();

    TypedEdgeGraph {
        nodes: sorted_nodes,
        outgoing,
        unresolved_sources,
    }
}

/// Dedup key for inherited facts — `op|resourceKind|resourceId|confidence`.
/// Mirrors `inheritedFactKey`.
fn inherited_fact_key(f: &CapabilityFact) -> String {
    format!(
        "{}|{}|{}|{}",
        f.op,
        f.resource_kind,
        f.resource_id.as_deref().unwrap_or(""),
        f.confidence
    )
}

/// Serialize a ValueSource to a stable JSON-ish string for `repKey`
/// (`JSON.stringify(resourceArgSource ?? null)` parity). Uses serde_json on a
/// canonical projection so the byte form matches al-sem's `JSON.stringify`.
fn value_source_json(vs: &Option<ValueSource>) -> String {
    match vs {
        None => "null".to_string(),
        Some(v) => serde_json::to_string(&value_source_to_json(v)).unwrap_or_default(),
    }
}

pub(crate) fn value_source_to_json(vs: &ValueSource) -> serde_json::Value {
    use serde_json::json;
    match vs {
        ValueSource::Literal { value } => json!({"kind":"literal","value":value}),
        ValueSource::Enum { enum_name, member } => match member {
            Some(m) => json!({"kind":"enum","enumName":enum_name,"member":m}),
            None => json!({"kind":"enum","enumName":enum_name}),
        },
        ValueSource::ConstantVar {
            var_name,
            initializer,
        } => {
            json!({"kind":"constant-var","varName":var_name,"initializer":value_source_to_json(initializer)})
        }
        ValueSource::Parameter { index, var_name } => {
            json!({"kind":"parameter","index":index,"varName":var_name})
        }
        ValueSource::TableField {
            table_id,
            field_name,
        } => json!({"kind":"table-field","tableId":table_id,"fieldName":field_name}),
        ValueSource::Expression => json!({"kind":"expression"}),
        ValueSource::Unknown => json!({"kind":"unknown"}),
    }
}

/// Serialize a CapabilityExtra to a stable JSON string for `repKey`
/// (`JSON.stringify(extra ?? null)` parity).
fn extra_json(extra: &Option<CapabilityExtra>) -> String {
    match extra {
        None => "null".to_string(),
        Some(e) => serde_json::to_string(&extra_to_json(e)).unwrap_or_default(),
    }
}

pub(crate) fn extra_to_json(e: &CapabilityExtra) -> serde_json::Value {
    use serde_json::json;
    match e {
        CapabilityExtra::Table {
            record_variable_id,
            temp_state,
            op_subtype,
        } => {
            let mut m = serde_json::Map::new();
            m.insert("kind".into(), json!("table"));
            if let Some(rv) = record_variable_id {
                m.insert("recordVariableId".into(), json!(rv));
            }
            if let Some(ts) = temp_state {
                m.insert("tempState".into(), temp_state_to_json(ts));
            }
            if let Some(os) = op_subtype {
                m.insert("opSubtype".into(), json!(os));
            }
            serde_json::Value::Object(m)
        }
        CapabilityExtra::Dispatch { object_type, modal } => {
            let mut m = serde_json::Map::new();
            m.insert("kind".into(), json!("dispatch"));
            m.insert("objectType".into(), json!(object_type));
            if let Some(md) = modal {
                m.insert("modal".into(), json!(md));
            }
            serde_json::Value::Object(m)
        }
        CapabilityExtra::Event {
            event_class,
            include_sender,
        } => {
            let mut m = serde_json::Map::new();
            m.insert("kind".into(), json!("event"));
            m.insert("eventClass".into(), json!(event_class));
            if let Some(is) = include_sender {
                m.insert("includeSender".into(), json!(is));
            }
            serde_json::Value::Object(m)
        }
        CapabilityExtra::Http {
            method,
            body_arg_source,
        } => {
            let mut m = serde_json::Map::new();
            m.insert("kind".into(), json!("http"));
            m.insert("method".into(), json!(method));
            if let Some(bs) = body_arg_source {
                m.insert("bodyArgSource".into(), value_source_to_json(bs));
            }
            serde_json::Value::Object(m)
        }
        CapabilityExtra::Storage {
            key_arg_source,
            value_arg_source,
            scope,
        } => {
            let mut m = serde_json::Map::new();
            m.insert("kind".into(), json!("storage"));
            if let Some(ks) = key_arg_source {
                m.insert("keyArgSource".into(), value_source_to_json(ks));
            }
            if let Some(vs) = value_arg_source {
                m.insert("valueArgSource".into(), value_source_to_json(vs));
            }
            if let Some(sc) = scope {
                m.insert("scope".into(), json!(sc));
            }
            serde_json::Value::Object(m)
        }
    }
}

fn temp_state_to_json(ts: &crate::engine::l2::features::PTempState) -> serde_json::Value {
    use serde_json::json;
    match ts.kind.as_str() {
        "known" => json!({"kind":"known","value": ts.value.unwrap_or(false)}),
        "parameter-dependent" => {
            json!({"kind":"parameter-dependent","parameterIndex": ts.parameter_index.unwrap_or(0)})
        }
        _ => json!({"kind":"unknown"}),
    }
}

/// Canonical representative key — deterministic, traversal-order-independent.
/// Mirrors `repKey` (the `§`-joined tuple).
fn rep_key(f: &CapabilityFact) -> String {
    [
        inherited_fact_key(f),
        f.subject.clone(),
        f.witness_operation_id.clone().unwrap_or_default(),
        f.witness_callsite_id.clone().unwrap_or_default(),
        value_source_json(&f.resource_arg_source),
        extra_json(&f.extra),
    ]
    .join("§")
}

/// One cone fact entry: the representative DIRECT fact + its min hop distance.
#[derive(Debug, Clone)]
struct ConeFactEntry {
    rep: CapabilityFact,
    dist: usize,
}

/// A fact cone: dedup key → entry (min dist, tie-broken by canonical rep).
type ConeFacts = BTreeMap<String, ConeFactEntry>;

/// Merge `entry` (already distance-shifted) into `dst` at `key`, keeping min
/// dist; tie-break by canonical rep. Mirrors `mergeCone`.
fn merge_cone(dst: &mut ConeFacts, key: String, entry: ConeFactEntry) {
    match dst.get(&key) {
        None => {
            dst.insert(key, entry);
        }
        Some(existing) => {
            // min dist wins; equal dist → smaller canonical rep wins (mergeCone).
            let wins = entry.dist < existing.dist
                || (entry.dist == existing.dist && rep_key(&entry.rep) < rep_key(&existing.rep));
            if wins {
                dst.insert(key, entry);
            }
        }
    }
}

/// Re-tag a representative direct fact as an inherited fact on `subject` via a
/// first-hop edge. Mirrors `retag`.
fn retag(rep: &CapabilityFact, subject: &str, edge: &TypedOutEdge) -> CapabilityFact {
    CapabilityFact {
        subject: subject.to_string(),
        provenance: "inherited".to_string(),
        via: capability_via_for_edge_kind(&edge.kind).to_string(),
        witness_callsite_id: edge.callsite.clone(),
        ..rep.clone()
    }
}

/// Sort key for the final capabilityFactsInherited array. Mirrors
/// `inheritedOutputSortKey`.
fn inherited_output_sort_key(f: &CapabilityFact) -> String {
    [
        f.op.clone(),
        f.resource_kind.clone(),
        f.resource_id.clone().unwrap_or_default(),
        f.confidence.clone(),
        f.via.clone(),
        f.witness_callsite_id.clone().unwrap_or_default(),
        f.witness_operation_id.clone().unwrap_or_default(),
    ]
    .join("|")
}

fn sort_inherited(mut facts: Vec<CapabilityFact>) -> Vec<CapabilityFact> {
    facts.sort_by_key(inherited_output_sort_key);
    facts
}

/// Per-routine direct facts grouped by dedup key (canonical rep per key).
type RoutineDirectFacts = HashMap<String, BTreeMap<String, CapabilityFact>>;

/// Minimal per-routine direct coverage the cone roll-up needs.
struct DirectCoverage {
    direct_status: String,
    reasons: Vec<String>,
}
type RoutineDirectCoverage = HashMap<String, DirectCoverage>;

/// A coverage cone (includes self). Mirrors `CoverageCone`.
#[derive(Debug, Clone)]
struct CoverageCone {
    complete: bool,
    reasons: Vec<String>,
    unknown_targets: Vec<String>,
}

/// Singleton non-recursive fast path. Mirrors `inheritedFactsForSingleton`.
fn inherited_facts_for_singleton(
    subject: &str,
    g: &TypedEdgeGraph,
    scc_id_by_routine: &HashMap<String, usize>,
    cones: &HashMap<usize, ConeFacts>,
) -> Vec<CapabilityFact> {
    struct Best {
        rep: CapabilityFact,
        dist: usize,
        edge: TypedOutEdge,
    }
    let mut best: BTreeMap<String, Best> = BTreeMap::new();
    let empty: Vec<TypedOutEdge> = Vec::new();
    for edge in g.outgoing.get(subject).unwrap_or(&empty) {
        let Some(yj) = scc_id_by_routine.get(&edge.to) else {
            continue;
        };
        let Some(ycone) = cones.get(yj) else {
            continue;
        };
        for (key, entry) in ycone {
            let cand_dist = entry.dist + 1;
            // min dist wins; equal dist → smaller edgeSortKey wins (the
            // first-hop tie-breaker). Mirrors inheritedFactsForSingleton.
            let wins = match best.get(key) {
                None => true,
                Some(cur) => {
                    cand_dist < cur.dist
                        || (cand_dist == cur.dist && edge_sort_key(edge) < edge_sort_key(&cur.edge))
                }
            };
            if wins {
                best.insert(
                    key.clone(),
                    Best {
                        rep: entry.rep.clone(),
                        dist: cand_dist,
                        edge: edge.clone(),
                    },
                );
            }
        }
    }
    let out: Vec<CapabilityFact> = best
        .values()
        .map(|b| retag(&b.rep, subject, &b.edge))
        .collect();
    sort_inherited(out)
}

/// Set-correct path for routines in recursive SCCs. Mirrors
/// `inheritedFactsByBfs`.
fn inherited_facts_by_bfs(
    subject: &str,
    g: &TypedEdgeGraph,
    direct: &RoutineDirectFacts,
    scc_id_by_routine: &HashMap<String, usize>,
    cones: &HashMap<usize, ConeFacts>,
) -> Vec<CapabilityFact> {
    let my_scc = scc_id_by_routine.get(subject).copied();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<CapabilityFact> = Vec::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(subject.to_string());

    struct QI {
        id: String,
        first_hop: TypedOutEdge,
    }
    let mut queue: std::collections::VecDeque<QI> = std::collections::VecDeque::new();
    let empty: Vec<TypedOutEdge> = Vec::new();
    for edge in g.outgoing.get(subject).unwrap_or(&empty) {
        if !visited.contains(&edge.to) {
            visited.insert(edge.to.clone());
            queue.push_back(QI {
                id: edge.to.clone(),
                first_hop: edge.clone(),
            });
        }
    }
    while let Some(item) = queue.pop_front() {
        let id = item.id;
        let first_hop = item.first_hop;
        if scc_id_by_routine.get(&id).copied() == my_scc {
            // sibling member: emit its own direct facts (attributed to firstHop).
            if let Some(byk) = direct.get(&id) {
                for (key, rep) in byk {
                    if !seen.contains(key) {
                        seen.insert(key.clone());
                        out.push(retag(rep, subject, &first_hop));
                    }
                }
            }
            for edge in g.outgoing.get(&id).unwrap_or(&empty) {
                if !visited.contains(&edge.to) {
                    visited.insert(edge.to.clone());
                    queue.push_back(QI {
                        id: edge.to.clone(),
                        first_hop: first_hop.clone(),
                    });
                }
            }
        } else {
            // downstream entry: pull its whole cone, do NOT recurse.
            let ycone = scc_id_by_routine.get(&id).and_then(|yj| cones.get(yj));
            if let Some(ycone) = ycone {
                for (key, entry) in ycone {
                    if !seen.contains(key) {
                        seen.insert(key.clone());
                        out.push(retag(&entry.rep, subject, &first_hop));
                    }
                }
            }
        }
    }
    sort_inherited(out)
}

/// Distinct successor SCCs per SCC (cross-SCC edges only). Mirrors
/// `buildSuccSccs`.
fn build_succ_sccs(
    g: &TypedEdgeGraph,
    sccs: &[Scc],
    scc_id_by_routine: &HashMap<String, usize>,
) -> Vec<BTreeSet<usize>> {
    let mut succ: Vec<BTreeSet<usize>> = sccs.iter().map(|_| BTreeSet::new()).collect();
    let empty: Vec<TypedOutEdge> = Vec::new();
    for (i, scc) in sccs.iter().enumerate() {
        for m in &scc.members {
            for e in g.outgoing.get(m).unwrap_or(&empty) {
                if let Some(yj) = scc_id_by_routine.get(&e.to) {
                    if *yj != i {
                        succ[i].insert(*yj);
                    }
                }
            }
        }
    }
    succ
}

/// Build one SCC's fact cone (members at dist 0 + successor cones at dist+1).
/// Mirrors `factConeForScc`.
fn fact_cone_for_scc(
    members: &[String],
    succ_ids: &BTreeSet<usize>,
    fact_cones: &HashMap<usize, ConeFacts>,
    direct: &RoutineDirectFacts,
) -> ConeFacts {
    let mut cone: ConeFacts = BTreeMap::new();
    for m in members {
        if let Some(byk) = direct.get(m) {
            for (key, f) in byk {
                merge_cone(
                    &mut cone,
                    key.clone(),
                    ConeFactEntry {
                        rep: f.clone(),
                        dist: 0,
                    },
                );
            }
        }
    }
    for y in succ_ids {
        if let Some(yc) = fact_cones.get(y) {
            for (key, entry) in yc {
                merge_cone(
                    &mut cone,
                    key.clone(),
                    ConeFactEntry {
                        rep: entry.rep.clone(),
                        dist: entry.dist + 1,
                    },
                );
            }
        }
    }
    cone
}

/// Build one SCC's coverage cone (includes self). Mirrors `coverageConeForScc`.
fn coverage_cone_for_scc(
    members: &[String],
    succ_ids: &BTreeSet<usize>,
    cov_cones: &HashMap<usize, CoverageCone>,
    cov: &RoutineDirectCoverage,
    unresolved_sources: &BTreeSet<String>,
) -> CoverageCone {
    let mut complete = true;
    let mut reason_set: BTreeSet<String> = BTreeSet::new();
    let mut unknown_set: BTreeSet<String> = BTreeSet::new();
    for m in members {
        if unresolved_sources.contains(m) {
            complete = false;
            reason_set.insert("object-run-unresolved".to_string());
        }
        if let Some(c) = cov.get(m) {
            if c.direct_status == "partial" || c.direct_status == "unknown" {
                complete = false;
                unknown_set.insert(m.clone());
                for r in &c.reasons {
                    reason_set.insert(r.clone());
                }
            }
        }
    }
    for y in succ_ids {
        if let Some(yc) = cov_cones.get(y) {
            if !yc.complete {
                complete = false;
            }
            for r in &yc.reasons {
                reason_set.insert(r.clone());
            }
            for t in &yc.unknown_targets {
                unknown_set.insert(t.clone());
            }
        }
    }
    CoverageCone {
        complete,
        reasons: reason_set.into_iter().collect(),
        unknown_targets: unknown_set.into_iter().collect(),
    }
}

/// The per-routine cone result.
struct InheritedConeResult {
    inherited: Vec<CapabilityFact>,
    coverage: CoverageRecord,
}

/// Compute capabilityFactsInherited + coverage for EVERY routine via a single
/// fused bottom-up SCC-cone pass. Mirrors `composeInheritedCones`. The engine
/// never throws: on internal inconsistency it still returns whatever it built.
#[allow(clippy::too_many_arguments)]
fn compose_inherited_cones(
    g: &TypedEdgeGraph,
    scc: &SccResult,
    direct: &RoutineDirectFacts,
    cov: &RoutineDirectCoverage,
    routine_ids: &BTreeSet<String>,
) -> HashMap<String, InheritedConeResult> {
    let mut out: HashMap<String, InheritedConeResult> = HashMap::new();

    let succ_sccs = build_succ_sccs(g, &scc.sccs, &scc.scc_id_by_routine);
    let mut remaining_uses: Vec<usize> = scc.sccs.iter().map(|_| 0usize).collect();
    for set in &succ_sccs {
        for y in set {
            remaining_uses[*y] += 1;
        }
    }

    let mut fact_cones: HashMap<usize, ConeFacts> = HashMap::new();
    let mut cov_cones: HashMap<usize, CoverageCone> = HashMap::new();

    let empty_succ: BTreeSet<usize> = BTreeSet::new();
    for i in 0..scc.sccs.len() {
        let scc_entry = &scc.sccs[i];
        let succ_ids = succ_sccs.get(i).unwrap_or(&empty_succ);

        let fcone = fact_cone_for_scc(&scc_entry.members, succ_ids, &fact_cones, direct);
        fact_cones.insert(i, fcone);
        let ccone = coverage_cone_for_scc(
            &scc_entry.members,
            succ_ids,
            &cov_cones,
            cov,
            &g.unresolved_sources,
        );
        cov_cones.insert(i, ccone.clone());

        let recursive = scc_entry.recursive;
        for m in &scc_entry.members {
            if !routine_ids.contains(m) {
                continue;
            }
            let inherited = if recursive {
                inherited_facts_by_bfs(m, g, direct, &scc.scc_id_by_routine, &fact_cones)
            } else {
                inherited_facts_for_singleton(m, g, &scc.scc_id_by_routine, &fact_cones)
            };
            let d_status = cov
                .get(m)
                .map(|c| c.direct_status.clone())
                .unwrap_or_else(|| "unknown".to_string());
            out.insert(
                m.clone(),
                InheritedConeResult {
                    inherited,
                    coverage: CoverageRecord {
                        subject: m.clone(),
                        direct_status: d_status,
                        inherited_status: if ccone.complete {
                            "complete".to_string()
                        } else {
                            "partial".to_string()
                        },
                        reasons: ccone.reasons.clone(),
                        unknown_targets: ccone.unknown_targets.clone(),
                    },
                },
            );
        }

        // refcount-free downstream cones whose last predecessor (this SCC) is done.
        for y in succ_ids {
            if remaining_uses[*y] > 0 {
                remaining_uses[*y] -= 1;
            }
            if remaining_uses[*y] == 0 {
                fact_cones.remove(y);
                cov_cones.remove(y);
            }
        }
    }

    out
}

// ===========================================================================
// R3b WRAP SEAM — a public cone entry the Salsa `inherited_facts`/`coverage`
// queries call. WRAPS the full cone (typed-edge graph build + Tarjan SCC +
// `compose_inherited_cones`) over a CombinedGraph + per-routine direct facts +
// direct coverage. Byte-identical to the cone path inside `project_r3a3` /
// `project_r3a5_cross_app` (it IS that path, factored out). No re-port.
// ===========================================================================

/// One routine's cone result in PUBLIC (internal-id) form. Field-for-field the
/// private `InheritedConeResult`.
#[derive(Debug, Clone, PartialEq, salsa::Update)]
pub struct ConeResultPub {
    pub inherited: Vec<CapabilityFact>,
    pub coverage: CoverageRecord,
}

/// Compute every routine's `capabilityFactsInherited` + `coverage` over the given
/// combined graph (typed edges already folded, incl. any injected intra-app dep
/// edges) + the per-routine direct facts + direct coverage. The `nodes` list is
/// the routine universe; `direct_in` maps routineId → its direct facts (ordered),
/// `coverage_in` maps routineId → `(direct_status, reasons)`.
///
/// This is the EXACT cone substrate `project_r3a3` / `project_r3a5_cross_app`
/// build inline; factored out so the R3b Salsa layer wraps it without re-porting.
pub fn compose_cone_over_graph(
    graph: &CombinedGraph,
    nodes: &[String],
    direct_in: &HashMap<String, Vec<CapabilityFact>>,
    coverage_in: &HashMap<String, (String, Vec<String>)>,
) -> HashMap<String, ConeResultPub> {
    let g = build_typed_edge_graph(graph, nodes);
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, list) in &g.outgoing {
        adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let scc = tarjan_scc(&SccInputGraph {
        nodes: &g.nodes,
        edges_by_from: &adjacency,
    });

    // Per-routine dedup-keyed direct facts (canonical rep per key) + direct
    // coverage + the routine-id set — mirrors the assembly in project_r3a3.
    let mut direct: RoutineDirectFacts = HashMap::new();
    let mut cov: RoutineDirectCoverage = HashMap::new();
    let mut routine_ids: BTreeSet<String> = BTreeSet::new();
    for id in nodes {
        routine_ids.insert(id.clone());
        if let Some(facts) = direct_in.get(id) {
            let mut byk: BTreeMap<String, CapabilityFact> = BTreeMap::new();
            for f in facts {
                let k = inherited_fact_key(f);
                match byk.get(&k) {
                    Some(cur) if rep_key(f) >= rep_key(cur) => {}
                    _ => {
                        byk.insert(k, f.clone());
                    }
                }
            }
            if !byk.is_empty() {
                direct.insert(id.clone(), byk);
            }
        }
        if let Some((status, reasons)) = coverage_in.get(id) {
            cov.insert(
                id.clone(),
                DirectCoverage {
                    direct_status: status.clone(),
                    reasons: reasons.clone(),
                },
            );
        }
    }

    let cones = compose_inherited_cones(&g, &scc, &direct, &cov, &routine_ids);
    cones
        .into_iter()
        .map(|(id, r)| {
            (
                id,
                ConeResultPub {
                    inherited: r.inherited,
                    coverage: r.coverage,
                },
            )
        })
        .collect()
}

// ===========================================================================
// R3a-3 STABLE PROJECTION — mirrors scripts/r3a3-projection.ts `projectR3a3`.
// ===========================================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum PValueSource {
    #[serde(rename = "literal")]
    Literal { value: String },
    #[serde(rename = "enum")]
    Enum {
        #[serde(rename = "enumName")]
        enum_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        member: Option<String>,
    },
    #[serde(rename = "constant-var")]
    ConstantVar {
        #[serde(rename = "varName")]
        var_name: String,
        initializer: Box<PValueSource>,
    },
    #[serde(rename = "parameter")]
    Parameter {
        index: u32,
        #[serde(rename = "varName")]
        var_name: String,
    },
    #[serde(rename = "table-field")]
    TableField {
        #[serde(rename = "tableId")]
        table_id: String,
        #[serde(rename = "fieldName")]
        field_name: String,
    },
    #[serde(rename = "expression")]
    Expression,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PCapabilityFact {
    pub op: String,
    #[serde(rename = "resourceKind")]
    pub resource_kind: String,
    pub confidence: String,
    pub provenance: String,
    pub via: String,
    #[serde(rename = "resourceId", skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(rename = "resourceArgSource", skip_serializing_if = "Option::is_none")]
    pub resource_arg_source: Option<PValueSource>,
    #[serde(rename = "witnessOperationId", skip_serializing_if = "Option::is_none")]
    pub witness_operation_id: Option<String>,
    #[serde(rename = "witnessCallsiteId", skip_serializing_if = "Option::is_none")]
    pub witness_callsite_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PCoverageRecord {
    pub subject: String,
    #[serde(rename = "directStatus")]
    pub direct_status: String,
    #[serde(rename = "inheritedStatus")]
    pub inherited_status: String,
    pub reasons: Vec<String>,
    #[serde(rename = "unknownTargets")]
    pub unknown_targets: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PRoutineConeCoverage {
    #[serde(rename = "routineId")]
    pub routine_id: String,
    #[serde(rename = "capabilityFactsDirect")]
    pub capability_facts_direct: Vec<PCapabilityFact>,
    #[serde(rename = "capabilityFactsInherited")]
    pub capability_facts_inherited: Vec<PCapabilityFact>,
    pub coverage: PCoverageRecord,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct R3a3Projection {
    pub summaries: Vec<PRoutineConeCoverage>,
}

/// Internal RoutineId → StableRoutineId; pass through if unmapped.
pub(crate) fn stable_routine_id(internal: &str, map: &HashMap<String, String>) -> String {
    map.get(internal)
        .cloned()
        .unwrap_or_else(|| internal.to_string())
}

/// Rewrite `${routineId}/<suffix>` → stable form (the suffix is everything after
/// the SECOND `/`). Mirrors `stableSubId`.
fn stable_sub_id(internal_sub_id: &str, map: &HashMap<String, String>) -> String {
    let first = internal_sub_id.find('/');
    let second = first.and_then(|f| internal_sub_id[f + 1..].find('/').map(|s| f + 1 + s));
    match second {
        Some(sec) => {
            let routine_id = &internal_sub_id[..sec];
            let suffix = &internal_sub_id[sec..];
            match map.get(routine_id) {
                Some(stable) => format!("{stable}{suffix}"),
                None => internal_sub_id.to_string(),
            }
        }
        None => internal_sub_id.to_string(),
    }
}

/// Internal TableId (`appGuid/table/N`) → StableTableId (`appGuid:Table:N`).
fn stable_table_id(internal: &str) -> String {
    if internal == "unknown" {
        return "unknown".to_string();
    }
    let parts: Vec<&str> = internal.split('/').collect();
    if parts.len() == 3 && parts[1] == "table" {
        format!("{}:Table:{}", parts[0], parts[2])
    } else {
        internal.to_string()
    }
}

/// Project an internal EventId to StableEventId via the EventSymbol. Mirrors
/// `stableEventId` (dumb `/`→`:` fallback when the symbol is absent).
fn stable_event_id(internal: &str, event_by_id: &HashMap<String, &EventSymbol>) -> String {
    match event_by_id.get(internal) {
        Some(evt) => format!(
            "{}::{}::{}",
            to_stable_object_id(&evt.publisher_object_id),
            evt.event_name,
            evt.signature_hash
        ),
        None => internal.replace('/', ":"),
    }
}

/// Project a CapabilityFact.resourceId to stable form (by resourceKind). Mirrors
/// `stableResourceId`.
fn stable_resource_id(
    internal: &str,
    resource_kind: &str,
    event_by_id: &HashMap<String, &EventSymbol>,
) -> String {
    match resource_kind {
        "table" | "transaction" => {
            if internal.contains("/table/") {
                stable_table_id(internal)
            } else {
                internal.replace('/', ":")
            }
        }
        "event" => stable_event_id(internal, event_by_id),
        "codeunit" | "page" | "report" => to_stable_object_id(internal),
        _ => internal.replace('/', ":"),
    }
}

/// Project an internal ValueSource → stable form (table-field tableId → stable).
fn project_value_source(vs: &ValueSource) -> PValueSource {
    match vs {
        ValueSource::Literal { value } => PValueSource::Literal {
            value: value.clone(),
        },
        ValueSource::Enum { enum_name, member } => PValueSource::Enum {
            enum_name: enum_name.clone(),
            member: member.clone(),
        },
        ValueSource::ConstantVar {
            var_name,
            initializer,
        } => PValueSource::ConstantVar {
            var_name: var_name.clone(),
            initializer: Box::new(project_value_source(initializer)),
        },
        ValueSource::Parameter { index, var_name } => PValueSource::Parameter {
            index: *index,
            var_name: var_name.clone(),
        },
        ValueSource::TableField {
            table_id,
            field_name,
        } => PValueSource::TableField {
            table_id: if table_id == "unknown" {
                "unknown".to_string()
            } else {
                stable_table_id(table_id)
            },
            field_name: field_name.clone(),
        },
        ValueSource::Expression => PValueSource::Expression,
        ValueSource::Unknown => PValueSource::Unknown,
    }
}

pub(crate) fn project_capability_fact(
    f: &CapabilityFact,
    map: &HashMap<String, String>,
    event_by_id: &HashMap<String, &EventSymbol>,
) -> PCapabilityFact {
    PCapabilityFact {
        op: f.op.clone(),
        resource_kind: f.resource_kind.clone(),
        confidence: f.confidence.clone(),
        provenance: f.provenance.clone(),
        via: f.via.clone(),
        resource_id: f
            .resource_id
            .as_ref()
            .map(|r| stable_resource_id(r, &f.resource_kind, event_by_id)),
        resource_arg_source: f.resource_arg_source.as_ref().map(project_value_source),
        witness_operation_id: f
            .witness_operation_id
            .as_ref()
            .map(|w| stable_sub_id(w, map)),
        witness_callsite_id: f
            .witness_callsite_id
            .as_ref()
            .map(|w| stable_sub_id(w, map)),
        extra: f.extra.as_ref().map(extra_to_json),
    }
}

/// Sort key for projected capability facts (al-sem `capabilityFactSortKey`).
pub(crate) fn capability_fact_sort_key(f: &PCapabilityFact) -> String {
    [
        f.op.clone(),
        f.resource_kind.clone(),
        f.resource_id.clone().unwrap_or_default(),
        f.confidence.clone(),
        f.via.clone(),
        f.witness_callsite_id.clone().unwrap_or_default(),
        f.witness_operation_id.clone().unwrap_or_default(),
    ]
    .join("|")
}

/// Map a routine's L4 fixed-point `uncertainties` to coverage reasons, faithful
/// to summary-runner.ts:565-596. Only the four call-resolution uncertainty kinds
/// that imply incomplete call resolution map to reasons:
///   ambiguous-overload → "ambiguous-overload"
///   member-not-found   → "member-not-found"
///   external-target    → "external-target"
///   interface-open-world → "interface-open-world"
/// (Source-only: `interfaceImplsKnowledgePartial` is false, so the coarser
/// `interface-impls-unknown-in-deps` reason never co-fires.)
///
/// Runs the L4 JACOBI fixed point over the COMBINED-graph SCC (the same inputs
/// `project_r3a2` uses) — the cone reads only the typed-edge SCC, but the
/// uncertainties are a property of the call-resolution fixed point.
fn compute_uncertainty_coverage_reasons(
    ws: &L3Workspace,
    graph: &CombinedGraph,
    calls: &crate::engine::l3::call_resolver::ResolvedCalls,
) -> HashMap<String, BTreeSet<String>> {
    use crate::engine::l4::summary_runner::{compute_summaries, FieldIndex};

    // Tarjan SCC over the COMBINED graph (summary substrate — distinct from the
    // typed-edge SCC the cone walks).
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, list) in &graph.edges_by_from {
        adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let scc = tarjan_scc(&SccInputGraph {
        nodes: &graph.nodes,
        edges_by_from: &adjacency,
    });

    // Field-resolution index (parameterRoles need it; harmless here).
    let mut field_index: FieldIndex = HashMap::new();
    for table in &ws.tables {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }

    let (summaries, _) = compute_summaries(
        &ws.routines,
        graph,
        &scc,
        &calls.upgraded_bindings,
        &field_index,
        false,
    );

    let mut out: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (rid, summary) in &summaries {
        let mut reasons: BTreeSet<String> = BTreeSet::new();
        for u in &summary.uncertainties {
            match u.kind.as_str() {
                "ambiguous-overload" => {
                    reasons.insert("ambiguous-overload".to_string());
                }
                "member-not-found" => {
                    reasons.insert("member-not-found".to_string());
                }
                "external-target" => {
                    reasons.insert("external-target".to_string());
                }
                "interface-open-world" => {
                    reasons.insert("interface-open-world".to_string());
                }
                _ => {}
            }
        }
        if !reasons.is_empty() {
            out.insert(rid.clone(), reasons);
        }
    }
    out
}

// ===========================================================================
// R3a-5 — the FULL cross-app RoutineSummary projection (core + cone/coverage).
//
// Ports al-sem's `analyzeWorkspace` order over the cross-app corpus WITH the dep
// hooks (`scripts/r3a5-projection.ts` `projectR3a5FullSummary`):
//   indexWorkspace(+deps) → withDependencyArtifacts → resolveModel
//   → buildCombinedGraph → injectIntraAppCallEdges → computeSummaries → cone
//
// What's new vs R3a-3 (source-only):
//   - The MERGED model carries dep routines (EMPTY merged features, like al-sem's
//     `EMPTY_FEATURES`) plus their RETAINED summary (the dep's own `via:"direct"`
//     dbEffects) + RETAINED direct capability facts (the dep's own intrinsic
//     facts), recovered from each dep's embedded-source analysis (the R3a-4
//     producer path). Dep routines are LEAVES (compute_summaries_with_leaves).
//   - injectIntraAppCallEdges adds the dep intra-app `direct-call` edges to the
//     typed-edge graph so the cone propagates the dep's `capabilityFactsDirect`
//     through intra-dep chains AND to primary callers.
// ===========================================================================

/// One projected FULL RoutineSummary (R3a-2 core + R3a-3 cone/coverage + the
/// `isDepRoutine` flag). Field order + key names mirror al-sem
/// `PRoutineFullSummary` (`scripts/r3a5-projection.ts`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PRoutineFullSummary {
    #[serde(rename = "routineId")]
    pub routine_id: String,
    #[serde(rename = "isDepRoutine")]
    pub is_dep_routine: bool,
    #[serde(rename = "dbEffects")]
    pub db_effects: Vec<crate::engine::l4::summary::PDbEffect>,
    pub uncertainties: Vec<crate::engine::l4::summary::PUncertainty>,
    #[serde(rename = "parameterRoles")]
    pub parameter_roles: Vec<crate::engine::l4::summary::PRecordRoleSummary>,
    #[serde(rename = "inRecursiveCycle")]
    pub in_recursive_cycle: bool,
    #[serde(rename = "hasUnresolvedCalls")]
    pub has_unresolved_calls: bool,
    #[serde(rename = "capabilityFactsDirect")]
    pub capability_facts_direct: Vec<PCapabilityFact>,
    #[serde(rename = "capabilityFactsInherited")]
    pub capability_facts_inherited: Vec<PCapabilityFact>,
    pub coverage: PCoverageRecord,
}

/// The full R3a-5 cross-app projection — per-routine full summary + the
/// anti-degenerate matrix counts. Mirrors al-sem `R3a5FullSummaryProjection`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct R3a5FullSummaryProjection {
    #[serde(rename = "fixtureName")]
    pub fixture_name: String,
    pub summaries: Vec<PRoutineFullSummary>,
    #[serde(rename = "primaryRoutinesWithInheritedDepFacts")]
    pub primary_routines_with_inherited_dep_facts: usize,
    #[serde(rename = "primaryRoutinesWithDepDbEffects")]
    pub primary_routines_with_dep_db_effects: usize,
    #[serde(rename = "coveragesWithOpaqueAppsReason")]
    pub coverages_with_opaque_apps_reason: usize,
    #[serde(rename = "totalCrossAppInheritedFacts")]
    pub total_cross_app_inherited_facts: usize,
}

/// Per-dep-routine RETAINED L4 facts, recovered from a dep `.app`'s embedded
/// source (the R3a-4 producer path). The dep routine arrives in the merged model
/// EMPTY-featured; these are folded back so the cone + dbEffect compose can
/// propagate them, exactly as al-sem retains `summary.dbEffects(via:"direct")` +
/// `summary.capabilityFactsDirect` on the dep artifact's routines.
struct DepRetained {
    /// internal dep routine id → its retained summary (direct dbEffects only).
    summaries: HashMap<String, crate::engine::l4::summary::RoutineSummary>,
    /// internal dep routine id → its retained direct capability facts.
    direct_facts: HashMap<String, Vec<CapabilityFact>>,
    /// internal dep routine id → (direct_status, reasons) for the coverage cone.
    direct_coverage: HashMap<String, (String, Vec<String>)>,
}

/// Recover the per-dep-routine retained L4 facts from a dep `.app`'s embedded
/// source. Re-runs the dep's isolated assemble+resolve (the R3a-4 producer path)
/// so the dep routine carries its REAL features, then derives:
///   - the RETAINED summary (`base_intraprocedural_summary` → direct dbEffects),
///   - the RETAINED direct capability facts (`direct_facts_for_routine`),
///   - the RETAINED direct coverage (status + reasons).
///
/// The internal routine ids are content+modelInstanceId-derived, so they MATCH
/// the merged cross-app model's dep routine ids (verified for the corpus).
fn recover_dep_retained(app_bytes: &[u8], model_instance_id: &str) -> DepRetained {
    use crate::engine::deps::app_manifest::parse_app_manifest_xml;
    use crate::engine::deps::app_package_zip::extract_navx_manifest_xml;
    use crate::engine::deps::dep_artifact_l4::iterate_embedded_source;
    use crate::engine::l3::l3_workspace::assemble_workspace_units;
    use crate::engine::l4::summary_runner::base_intraprocedural_summary;

    let mut retained = DepRetained {
        summaries: HashMap::new(),
        direct_facts: HashMap::new(),
        direct_coverage: HashMap::new(),
    };

    let Some(manifest_xml) = extract_navx_manifest_xml(app_bytes) else {
        return retained;
    };
    let manifest = parse_app_manifest_xml(&manifest_xml);
    if manifest.error.is_some() || manifest.identity.app_guid.is_empty() {
        return retained;
    }
    let app_guid = manifest.identity.app_guid.clone();
    let embedded = iterate_embedded_source(app_bytes);
    if embedded.is_empty() {
        // Symbol-only / no embedded source → no retained facts. The merged model's
        // bodyless dep routine yields its own opaque coverage via the cone path.
        return retained;
    }
    let units: Vec<(String, String)> = embedded
        .iter()
        .map(|f| {
            (
                format!("dep:{app_guid}:{}", f.relative_path),
                f.content.clone(),
            )
        })
        .collect();
    let mut ws: L3Workspace = assemble_workspace_units(&units, &app_guid, model_instance_id);
    crate::engine::l3::l3_workspace::resolve(&mut ws);

    // Publisher events (for the publisher-fact injection in direct_facts_for_routine).
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let event_graph: EventGraph = build_event_graph(&ws.routines, &symbols);
    let mut publisher_events_by_routine: HashMap<String, Vec<&EventSymbol>> = HashMap::new();
    for evt in &event_graph.events {
        if let Some(pr) = &evt.publisher_routine_id {
            publisher_events_by_routine
                .entry(pr.clone())
                .or_default()
                .push(evt);
        }
    }

    // Field index for the base summary's parameterRoles (harmless for the corpus).
    let mut field_index: crate::engine::l4::summary_runner::FieldIndex = HashMap::new();
    for table in &ws.tables {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }
    let routines_by_id: HashMap<String, &L3Routine> =
        ws.routines.iter().map(|r| (r.id.clone(), r)).collect();

    let empty_pub: Vec<&EventSymbol> = Vec::new();
    for r in &ws.routines {
        if r.app_guid != app_guid {
            continue;
        }
        // RETAINED summary: the dep's OWN intraprocedural facts. al-sem keeps only
        // `via:"direct"` dbEffects (`dependency-pipeline.ts:632`); base_intraprocedural
        // emits exactly those (every base dbEffect is via:"direct").
        let base = base_intraprocedural_summary(r, &routines_by_id, &field_index);
        retained.summaries.insert(r.id.clone(), base);

        // RETAINED direct capability facts + direct coverage (status + reasons).
        let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
        let (facts, status, reasons) = direct_facts_for_routine(r, pubs);
        retained.direct_facts.insert(r.id.clone(), facts);
        retained
            .direct_coverage
            .insert(r.id.clone(), (status, reasons));
    }

    retained
}

/// The FULLY-ASSEMBLED cross-app L4 BASE — every from-scratch intermediate the
/// R3a-5 projection (and the R3b Salsa wrap) consume, before the core/cone +
/// projection. Extracted so the R3b Salsa layer can build its fine-grained inputs
/// from EXACTLY the same base the from-scratch path uses (no divergent assembly).
pub(crate) struct R3a5CrossAppBase {
    pub ws_routines: Vec<L3Routine>,
    pub dep_routine_ids: BTreeSet<String>,
    /// The combined graph WITH the injected dep intra-app typed edges folded in
    /// (the cone substrate). The combined `edges_by_from` / `uncertainty_edges`
    /// drive the JACOBI; the `typed_edges` (incl. injected) drive the cone.
    pub graph: CombinedGraph,
    pub combined_scc: SccResult,
    pub field_index: crate::engine::l4::summary_runner::FieldIndex,
    pub upgraded_bindings: HashMap<String, Vec<crate::engine::l3::call_resolver::UpgradedBinding>>,
    pub event_graph: EventGraph,
    pub objects: Vec<crate::engine::l3::l3_workspace::L3Object>,
    pub tables: Vec<crate::engine::l3::l3_workspace::L3Table>,
    pub app_guid: String,
    /// Fixed-leaf (dep) RETAINED summaries.
    pub leaf_summaries: HashMap<String, crate::engine::l4::summary::RoutineSummary>,
    /// Per-routine direct capability facts (full, ordered).
    pub direct_full: HashMap<String, Vec<CapabilityFact>>,
    /// Per-routine direct coverage `(status, reasons)`.
    pub direct_coverage: HashMap<String, (String, Vec<String>)>,
    pub nodes: Vec<String>,
}

/// Assemble the cross-app L4 BASE (steps 1–3 + 5 of the from-scratch pipeline):
/// the merged model, the dep artifacts + recovered retained facts, the combined
/// graph (WITH injected dep intra-app typed edges), the combined-graph SCC, the
/// field index, and the per-routine direct facts/coverage. The core JACOBI, the
/// cone, and the projection all run OVER this base (by both the from-scratch path
/// and the R3b Salsa wrap). Returns `None` for a fail-closed / dep-less workspace.
pub(crate) fn build_r3a5_cross_app_base(
    workspace: &std::path::Path,
    model_instance_id: &str,
) -> Option<R3a5CrossAppBase> {
    use crate::engine::deps::cross_app_l3::build_cross_app_l3_from_workspace;
    use crate::engine::deps::dep_artifact_l4::{
        build_dep_artifact_l4, inject_intra_app_call_edges, ConsumerModel,
    };
    use crate::engine::deps::merged_index::collect_app_paths;
    use crate::engine::l4::summary_runner::FieldIndex;

    // --- 1. Merged cross-app L3. ---
    let mut cross = build_cross_app_l3_from_workspace(workspace, model_instance_id)?;

    // --- 2. Dep artifacts (injected intra-app edges) + recovered retained facts. ---
    let alpackages = workspace.join(".alpackages");
    let app_paths = collect_app_paths(&alpackages);
    let mut artifacts = Vec::new();
    let mut dep_retained = DepRetained {
        summaries: HashMap::new(),
        direct_facts: HashMap::new(),
        direct_coverage: HashMap::new(),
    };
    for p in &app_paths {
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        if let Some(a) = build_dep_artifact_l4(&bytes, model_instance_id) {
            artifacts.push(a);
        }
        let r = recover_dep_retained(&bytes, model_instance_id);
        dep_retained.summaries.extend(r.summaries);
        dep_retained.direct_facts.extend(r.direct_facts);
        dep_retained.direct_coverage.extend(r.direct_coverage);
    }

    // Restore source-bearing dep routines' bodyAvailable (parity, see project_r3a5).
    for r in &mut cross.resolved.workspace.routines {
        if dep_retained.summaries.contains_key(&r.id) {
            r.body_available = true;
        }
    }
    let ws: &L3Workspace = &cross.resolved.workspace;
    let fetched_lc: BTreeSet<String> = cross
        .fetched_app_guids
        .iter()
        .map(|g| g.to_lowercase())
        .collect();
    let dep_routine_ids: BTreeSet<String> = ws
        .routines
        .iter()
        .filter(|r| fetched_lc.contains(&r.app_guid.to_lowercase()))
        .map(|r| r.id.clone())
        .collect();

    // --- 3. Call resolution + combined graph over the MERGED model. ---
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let declared: Vec<DeclaredDependency> = cross
        .declared_dep_app_guids
        .iter()
        .map(|g| DeclaredDependency {
            app_guid: g.clone(),
        })
        .collect();
    let calls = resolve_calls(ws, &symbols, &declared, &cross.fetched_app_guids);
    let event_graph: EventGraph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    let nodes: Vec<String> = ws.routines.iter().map(|r| r.id.clone()).collect();
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, list) in &graph.edges_by_from {
        adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let combined_scc = tarjan_scc(&SccInputGraph {
        nodes: &graph.nodes,
        edges_by_from: &adjacency,
    });
    let mut field_index: FieldIndex = HashMap::new();
    for table in &ws.tables {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }

    // --- 5a. Fold the injected dep intra-app typed edges into the combined graph
    //         (the cone substrate). ---
    let mut consumer = ConsumerModel::with_routine_ids(nodes.clone());
    inject_intra_app_call_edges(&mut consumer, &artifacts);
    let mut graph_with_injected = graph;
    for e in &consumer.injected_typed_edges {
        graph_with_injected.typed_edges.push(TypedEdge {
            kind: e.kind.clone(),
            from: e.from.clone(),
            to: Some(e.to.clone()),
            callsite_id: Some(e.callsite_id.clone()),
            operation_id: None,
            event_id: None,
            receiver_type: None,
            interface_name: None,
            candidate_count: None,
            target_object: None,
            target_id_source: None,
            object_type: None,
        });
    }

    // Publisher events (for the workspace direct facts).
    let mut publisher_events_by_routine: HashMap<String, Vec<&EventSymbol>> = HashMap::new();
    for evt in &event_graph.events {
        if let Some(pr) = &evt.publisher_routine_id {
            publisher_events_by_routine
                .entry(pr.clone())
                .or_default()
                .push(evt);
        }
    }

    // --- 5b. Per-routine direct facts (full) + direct coverage. ---
    let mut direct_full: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
    let mut direct_coverage: HashMap<String, (String, Vec<String>)> = HashMap::new();
    let empty_pub: Vec<&EventSymbol> = Vec::new();
    for r in &ws.routines {
        let is_dep = dep_routine_ids.contains(&r.id);
        let (facts, status, reasons) = if is_dep {
            let facts = dep_retained
                .direct_facts
                .get(&r.id)
                .cloned()
                .unwrap_or_default();
            let (status, reasons) = dep_retained
                .direct_coverage
                .get(&r.id)
                .cloned()
                .unwrap_or_else(|| ("unknown".to_string(), vec!["opaque-dependency".to_string()]));
            (facts, status, reasons)
        } else {
            let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
            direct_facts_for_routine(r, pubs)
        };
        direct_coverage.insert(r.id.clone(), (status, reasons));
        direct_full.insert(r.id.clone(), facts);
    }

    // The primary (workspace) app guid — the "source" entry in the apps ledger,
    // else the first non-dep routine's app guid (cosmetic: the AppContext identity).
    let app_guid = cross
        .apps
        .iter()
        .find(|(_, kind)| kind == "source")
        .map(|(g, _)| g.clone())
        .or_else(|| {
            ws.routines
                .iter()
                .find(|r| !dep_routine_ids.contains(&r.id))
                .map(|r| r.app_guid.clone())
        })
        .unwrap_or_default();
    Some(R3a5CrossAppBase {
        ws_routines: ws.routines.clone(),
        dep_routine_ids,
        objects: ws.objects.clone(),
        tables: ws.tables.clone(),
        graph: graph_with_injected,
        combined_scc,
        field_index,
        upgraded_bindings: calls.upgraded_bindings.clone(),
        event_graph,
        app_guid,
        leaf_summaries: dep_retained.summaries,
        direct_full,
        direct_coverage,
        nodes,
    })
}

/// Run the FULL cross-app L4 summary pipeline (merged model + dep hooks + cone)
/// and project every routine's full RoutineSummary in stable form. The R3a-5
/// EXIT-GATE differential surface. Engine-never-throws: a fail-closed / dep-less
/// workspace yields an empty projection.
pub fn project_r3a5_cross_app(
    workspace: &std::path::Path,
    model_instance_id: &str,
    fixture_name: &str,
) -> R3a5FullSummaryProjection {
    use crate::engine::l4::summary_runner::compute_summaries_with_leaves;

    let empty = R3a5FullSummaryProjection {
        fixture_name: fixture_name.to_string(),
        summaries: Vec::new(),
        primary_routines_with_inherited_dep_facts: 0,
        primary_routines_with_dep_db_effects: 0,
        coverages_with_opaque_apps_reason: 0,
        total_cross_app_inherited_facts: 0,
    };

    let Some(base) = build_r3a5_cross_app_base(workspace, model_instance_id) else {
        return empty;
    };
    let ws_routines = &base.ws_routines;
    let dep_routine_ids = &base.dep_routine_ids;
    let graph = &base.graph;
    let event_graph = &base.event_graph;
    let direct_full = &base.direct_full;

    // From-scratch core (JACOBI) + cone over the assembled base.
    let (core_summaries, _) = compute_summaries_with_leaves(
        ws_routines,
        graph,
        &base.combined_scc,
        &base.upgraded_bindings,
        &base.field_index,
        false,
        &base.leaf_summaries,
    );
    let cones =
        compose_cone_over_graph(graph, &base.nodes, &base.direct_full, &base.direct_coverage);

    project_r3a5_from_parts(
        ws_routines,
        dep_routine_ids,
        &core_summaries,
        &cones,
        direct_full,
        event_graph,
        fixture_name,
    )
}

/// The R3a-5 PROJECTION TAIL — project the (core JACOBI summaries + cone) over the
/// merged model into the stable full-RoutineSummary surface + the anti-degenerate
/// counts. A pure function of its parts, shared by the from-scratch
/// `project_r3a5_cross_app` AND the R3b Salsa wrap (which demands the
/// `core_summaries` and `cones` through the Salsa queries — proving the wrapped
/// projection is byte-identical to the from-scratch one).
#[allow(clippy::too_many_arguments)]
pub(crate) fn project_r3a5_from_parts(
    ws_routines: &[L3Routine],
    dep_routine_ids: &BTreeSet<String>,
    core_summaries: &HashMap<String, crate::engine::l4::summary::RoutineSummary>,
    cones: &HashMap<String, ConeResultPub>,
    direct_full: &HashMap<String, Vec<CapabilityFact>>,
    event_graph: &EventGraph,
    fixture_name: &str,
) -> R3a5FullSummaryProjection {
    use crate::engine::l4::summary::project_routine_summary_core_pub;

    let map: HashMap<String, String> = ws_routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();
    let event_by_id: HashMap<String, &EventSymbol> = event_graph
        .events
        .iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    let mut summaries: Vec<PRoutineFullSummary> = Vec::new();
    for r in ws_routines {
        let is_dep = dep_routine_ids.contains(&r.id);

        // R3a-2 core (dbEffects / uncertainties / parameterRoles / cycle / unresolved).
        let core = core_summaries
            .get(&r.id)
            .map(|s| project_routine_summary_core_pub(s, &map));

        let (db_effects, uncertainties, parameter_roles, in_recursive_cycle, has_unresolved_calls) =
            match core {
                Some(c) => (
                    c.db_effects,
                    c.uncertainties,
                    c.parameter_roles,
                    c.in_recursive_cycle,
                    c.has_unresolved_calls,
                ),
                None => (Vec::new(), Vec::new(), Vec::new(), false, false),
            };

        // R3a-3 cone + coverage.
        let empty_facts: Vec<CapabilityFact> = Vec::new();
        let mut direct_facts: Vec<PCapabilityFact> = direct_full
            .get(&r.id)
            .unwrap_or(&empty_facts)
            .iter()
            .map(|f| project_capability_fact(f, &map, &event_by_id))
            .collect();
        direct_facts.sort_by_key(capability_fact_sort_key);

        let (inherited_facts, coverage) = match cones.get(&r.id) {
            Some(cone) => {
                let mut inh: Vec<PCapabilityFact> = cone
                    .inherited
                    .iter()
                    .map(|f| project_capability_fact(f, &map, &event_by_id))
                    .collect();
                inh.sort_by_key(capability_fact_sort_key);

                let mut reasons = cone.coverage.reasons.clone();
                reasons.sort();
                let mut unknown_targets: Vec<String> = cone
                    .coverage
                    .unknown_targets
                    .iter()
                    .map(|t| stable_routine_id(t, &map))
                    .collect();
                unknown_targets.sort();

                (
                    inh,
                    PCoverageRecord {
                        subject: stable_routine_id(&cone.coverage.subject, &map),
                        direct_status: cone.coverage.direct_status.clone(),
                        inherited_status: cone.coverage.inherited_status.clone(),
                        reasons,
                        unknown_targets,
                    },
                )
            }
            None => (
                Vec::new(),
                PCoverageRecord {
                    subject: stable_routine_id(&r.id, &map),
                    direct_status: "unknown".to_string(),
                    inherited_status: "unknown".to_string(),
                    reasons: Vec::new(),
                    unknown_targets: Vec::new(),
                },
            ),
        };

        summaries.push(PRoutineFullSummary {
            routine_id: stable_routine_id(&r.id, &map),
            is_dep_routine: is_dep,
            db_effects,
            uncertainties,
            parameter_roles,
            in_recursive_cycle,
            has_unresolved_calls,
            capability_facts_direct: direct_facts,
            capability_facts_inherited: inherited_facts,
            coverage,
        });
    }
    summaries.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));

    // --- Anti-degenerate matrix counts (over the PRIMARY routines). ---
    let primary: Vec<&PRoutineFullSummary> =
        summaries.iter().filter(|s| !s.is_dep_routine).collect();
    let primary_routines_with_inherited_dep_facts = primary
        .iter()
        .filter(|s| !s.capability_facts_inherited.is_empty())
        .count();
    let primary_routines_with_dep_db_effects =
        primary.iter().filter(|s| !s.db_effects.is_empty()).count();
    let coverages_with_opaque_apps_reason = summaries
        .iter()
        .filter(|s| {
            s.coverage.reasons.iter().any(|r| {
                r == "opaque-dependency" || r == "external-target" || r == "missing-dep-package"
            })
        })
        .count();
    let total_cross_app_inherited_facts: usize = primary
        .iter()
        .map(|s| s.capability_facts_inherited.len())
        .sum();

    R3a5FullSummaryProjection {
        fixture_name: fixture_name.to_string(),
        summaries,
        primary_routines_with_inherited_dep_facts,
        primary_routines_with_dep_db_effects,
        coverages_with_opaque_apps_reason,
        total_cross_app_inherited_facts,
    }
}

// ===========================================================================
// project_r3a3 — the L3Resolved entry point.
// ===========================================================================

/// Run the full source-only pipeline (call-resolve → combined graph → typed-edge
/// graph + SCC → direct facts → cone) and project the post-computeSummaries
/// cone+coverage surface. READ-once; no dep hooks; no JACOBI summary core
/// (the cone reads ONLY the direct facts + the typed edges, exactly as al-sem's
/// post-fixed-point cone pass does over the source-only model).
pub fn project_r3a3(resolved: &L3Resolved) -> R3a3Projection {
    let ws: &L3Workspace = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    let event_graph: EventGraph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    // Typed-edge graph (cone substrate) + Tarjan SCC over it.
    let nodes: Vec<String> = ws.routines.iter().map(|r| r.id.clone()).collect();
    let g = build_typed_edge_graph(&graph, &nodes);
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, list) in &g.outgoing {
        adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let scc = tarjan_scc(&SccInputGraph {
        nodes: &g.nodes,
        edges_by_from: &adjacency,
    });

    // Publisher events indexed by routine (for the publisher-fact injection).
    let mut publisher_events_by_routine: HashMap<String, Vec<&EventSymbol>> = HashMap::new();
    for evt in &event_graph.events {
        if let Some(pr) = &evt.publisher_routine_id {
            publisher_events_by_routine
                .entry(pr.clone())
                .or_default()
                .push(evt);
        }
    }

    // Per-routine UNCERTAINTY-DERIVED coverage reasons (summary-runner.ts:565-596).
    // Run the L4 fixed-point summary (over the COMBINED-graph SCC) to obtain each
    // routine's `uncertainties`, then map the four call-resolution uncertainty kinds
    // (ambiguous-overload / member-not-found / external-target / interface-open-world)
    // to coverage reasons. A routine carrying any of these has its directStatus
    // downgraded complete→partial so the coverage cone forwards the reason (+ adds
    // the routine to unknownTargets). Source-only `interfaceImplsKnowledgePartial`
    // is false, so the `interface-impls-unknown-in-deps` add-on never fires.
    let uncertainty_reasons = compute_uncertainty_coverage_reasons(ws, &graph, &calls);

    // Per-routine direct facts (full) + direct coverage + the dedup-keyed map.
    let mut direct_full: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
    let mut direct: RoutineDirectFacts = HashMap::new();
    let mut cov: RoutineDirectCoverage = HashMap::new();
    let mut routine_ids: BTreeSet<String> = BTreeSet::new();
    let empty_pub: Vec<&EventSymbol> = Vec::new();

    for r in &ws.routines {
        routine_ids.insert(r.id.clone());
        let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
        let (facts, mut status, reasons) = direct_facts_for_routine(r, pubs);

        // Canonical rep per dedup key (min repKey wins) — mirrors the
        // composeInheritedCones direct-fact dedup.
        let mut byk: BTreeMap<String, CapabilityFact> = BTreeMap::new();
        for f in &facts {
            let k = inherited_fact_key(f);
            match byk.get(&k) {
                Some(cur) if rep_key(f) >= rep_key(cur) => {}
                _ => {
                    byk.insert(k, f.clone());
                }
            }
        }
        if !byk.is_empty() {
            direct.insert(r.id.clone(), byk);
        }

        // Fold uncertainty-derived coverage reasons (summary-runner.ts:565-596).
        let mut reason_set: BTreeSet<String> = reasons.iter().cloned().collect();
        let base_len = reason_set.len();
        if let Some(extra_reasons) = uncertainty_reasons.get(&r.id) {
            for rr in extra_reasons {
                reason_set.insert(rr.clone());
            }
        }
        let final_reasons: Vec<String> = if reason_set.len() > base_len {
            // Uncertainty-derived reasons imply incomplete resolution → downgrade.
            if status == "complete" {
                status = "partial".to_string();
            }
            reason_set.into_iter().collect()
        } else {
            reasons.clone()
        };

        cov.insert(
            r.id.clone(),
            DirectCoverage {
                direct_status: status,
                reasons: final_reasons,
            },
        );
        direct_full.insert(r.id.clone(), facts);
    }

    let cones = compose_inherited_cones(&g, &scc, &direct, &cov, &routine_ids);

    // ── Project ───────────────────────────────────────────────────────────
    let map: HashMap<String, String> = ws
        .routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();
    let event_by_id: HashMap<String, &EventSymbol> = event_graph
        .events
        .iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    let mut summaries: Vec<PRoutineConeCoverage> = Vec::new();
    for r in &ws.routines {
        let Some(cone) = cones.get(&r.id) else {
            continue;
        };

        let empty_facts: Vec<CapabilityFact> = Vec::new();
        let mut direct_facts: Vec<PCapabilityFact> = direct_full
            .get(&r.id)
            .unwrap_or(&empty_facts)
            .iter()
            .map(|f| project_capability_fact(f, &map, &event_by_id))
            .collect();
        direct_facts.sort_by_key(capability_fact_sort_key);

        let mut inherited_facts: Vec<PCapabilityFact> = cone
            .inherited
            .iter()
            .map(|f| project_capability_fact(f, &map, &event_by_id))
            .collect();
        inherited_facts.sort_by_key(capability_fact_sort_key);

        let mut reasons = cone.coverage.reasons.clone();
        reasons.sort();
        let mut unknown_targets: Vec<String> = cone
            .coverage
            .unknown_targets
            .iter()
            .map(|t| stable_routine_id(t, &map))
            .collect();
        unknown_targets.sort();

        summaries.push(PRoutineConeCoverage {
            routine_id: stable_routine_id(&r.id, &map),
            capability_facts_direct: direct_facts,
            capability_facts_inherited: inherited_facts,
            coverage: PCoverageRecord {
                subject: stable_routine_id(&cone.coverage.subject, &map),
                direct_status: cone.coverage.direct_status.clone(),
                inherited_status: cone.coverage.inherited_status.clone(),
                reasons,
                unknown_targets,
            },
        });
    }
    summaries.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));

    R3a3Projection { summaries }
}

// ===========================================================================
// R4-F Stage-2b — SOURCE-ONLY cone/graph BASE for the CapabilitySnapshot.
//
// The snapshot's consumed-core derivers (capabilityFacts / typedEdges /
// operation+callsite evidence / callsiteResolutions / coverage / eventDeclarations)
// all RE-PROJECT the SAME source-only R3a-3 substrate the cone path assembles. This
// helper exposes that substrate's RAW (internal-id) parts so `engine::l5::snapshot`
// reshapes + rewrites-to-stable + sorts WITHOUT re-deriving any fact/edge/cone.
//
// Mirrors the inline assembly of `project_r3a3` (resolve_calls → build_event_graph
// → build_combined_graph → direct_facts_for_routine + uncertainty-folded coverage →
// compose_cone_over_graph), but returns the parts instead of projecting them.
// ===========================================================================

/// The RAW source-only L4 cone/graph base — every internal-id part the R4-F
/// snapshot derivers consume. Built by `build_r3a3_source_only_base`.
pub struct R3a3SourceBase {
    /// Workspace routines (clone) — the universe + per-routine features.
    pub ws_routines: Vec<L3Routine>,
    /// The combined graph (typed edges + call substrate).
    pub graph: CombinedGraph,
    /// The resolved call edges (for the callsite-resolution ledger).
    pub calls: crate::engine::l3::call_resolver::ResolvedCalls,
    /// The event graph (publisher symbols + subscriber edges).
    pub event_graph: EventGraph,
    /// Per-routine RAW direct capability facts (full, ordered — NOT deduped).
    pub direct_full: HashMap<String, Vec<CapabilityFact>>,
    /// Per-routine cone result (inherited facts + coverage), internal-id form.
    pub cones: HashMap<String, ConeResultPub>,
    /// Internal RoutineId → StableRoutineId.
    pub routine_to_stable: HashMap<String, String>,
}

/// Assemble the RAW source-only cone/graph base (mirrors `project_r3a3`'s inline
/// assembly, returning the parts). Fail-closed callers get an empty base via the
/// resolved workspace having zero routines.
pub fn build_r3a3_source_only_base(resolved: &L3Resolved) -> R3a3SourceBase {
    let ws: &L3Workspace = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    let event_graph: EventGraph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    // Publisher events indexed by routine (for the publisher-fact injection).
    let mut publisher_events_by_routine: HashMap<String, Vec<&EventSymbol>> = HashMap::new();
    for evt in &event_graph.events {
        if let Some(pr) = &evt.publisher_routine_id {
            publisher_events_by_routine
                .entry(pr.clone())
                .or_default()
                .push(evt);
        }
    }

    // Per-routine UNCERTAINTY-DERIVED coverage reasons (summary-runner.ts:565-596).
    let uncertainty_reasons = compute_uncertainty_coverage_reasons(ws, &graph, &calls);

    // Per-routine direct facts (full) + uncertainty-folded direct coverage.
    let mut direct_full: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
    let mut direct_coverage: HashMap<String, (String, Vec<String>)> = HashMap::new();
    let empty_pub: Vec<&EventSymbol> = Vec::new();
    for r in &ws.routines {
        let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
        let (facts, mut status, reasons) = direct_facts_for_routine(r, pubs);

        // Fold uncertainty-derived coverage reasons (identical to project_r3a3).
        let mut reason_set: BTreeSet<String> = reasons.iter().cloned().collect();
        let base_len = reason_set.len();
        if let Some(extra_reasons) = uncertainty_reasons.get(&r.id) {
            for rr in extra_reasons {
                reason_set.insert(rr.clone());
            }
        }
        let final_reasons: Vec<String> = if reason_set.len() > base_len {
            if status == "complete" {
                status = "partial".to_string();
            }
            reason_set.into_iter().collect()
        } else {
            reasons.clone()
        };

        direct_coverage.insert(r.id.clone(), (status, final_reasons));
        direct_full.insert(r.id.clone(), facts);
    }

    let nodes: Vec<String> = ws.routines.iter().map(|r| r.id.clone()).collect();
    let cones = compose_cone_over_graph(&graph, &nodes, &direct_full, &direct_coverage);

    let routine_to_stable: HashMap<String, String> = ws
        .routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();

    R3a3SourceBase {
        ws_routines: ws.routines.clone(),
        graph,
        calls,
        event_graph,
        direct_full,
        cones,
        routine_to_stable,
    }
}

/// The R3a-3 PROJECTION TAIL — project the per-routine cone results into the
/// stable cone/coverage surface. A pure function of its parts, shared by the
/// from-scratch `project_r3a3` AND the R3b Salsa wrap (which demands the cone via
/// the Salsa `cones` query). Only routines WITH a cone entry are emitted (parity
/// with `project_r3a3`). `event_graph` provides the stable event-id projection.
pub(crate) fn project_r3a3_from_parts(
    ws_routines: &[L3Routine],
    cones: &HashMap<String, ConeResultPub>,
    direct_full: &HashMap<String, Vec<CapabilityFact>>,
    event_graph: &EventGraph,
    map: &HashMap<String, String>,
) -> R3a3Projection {
    let event_by_id: HashMap<String, &EventSymbol> = event_graph
        .events
        .iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    let mut summaries: Vec<PRoutineConeCoverage> = Vec::new();
    for r in ws_routines {
        let Some(cone) = cones.get(&r.id) else {
            continue;
        };

        let empty_facts: Vec<CapabilityFact> = Vec::new();
        let mut direct_facts: Vec<PCapabilityFact> = direct_full
            .get(&r.id)
            .unwrap_or(&empty_facts)
            .iter()
            .map(|f| project_capability_fact(f, map, &event_by_id))
            .collect();
        direct_facts.sort_by_key(capability_fact_sort_key);

        let mut inherited_facts: Vec<PCapabilityFact> = cone
            .inherited
            .iter()
            .map(|f| project_capability_fact(f, map, &event_by_id))
            .collect();
        inherited_facts.sort_by_key(capability_fact_sort_key);

        let mut reasons = cone.coverage.reasons.clone();
        reasons.sort();
        let mut unknown_targets: Vec<String> = cone
            .coverage
            .unknown_targets
            .iter()
            .map(|t| stable_routine_id(t, map))
            .collect();
        unknown_targets.sort();

        summaries.push(PRoutineConeCoverage {
            routine_id: stable_routine_id(&r.id, map),
            capability_facts_direct: direct_facts,
            capability_facts_inherited: inherited_facts,
            coverage: PCoverageRecord {
                subject: stable_routine_id(&cone.coverage.subject, map),
                direct_status: cone.coverage.direct_status.clone(),
                inherited_status: cone.coverage.inherited_status.clone(),
                reasons,
                unknown_targets,
            },
        });
    }
    summaries.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));

    R3a3Projection { summaries }
}

// ===========================================================================
// REAL anti-degenerate matrix counts (review note 1). An INDEPENDENT oracle:
// re-derives the typed-edge graph + per-routine direct-fact keys, then runs a
// clean multi-source BFS over the typed edges to compute, per routine + per
// inheritedFactKey, the GENUINE shortest call-distance witness and whether ≥2
// DISTINCT first-hop edges reach that key at the same shortest distance (an
// equal-distance tie). It does NOT reuse the production singleton/BFS retag
// path, so it cross-validates the cone rather than echoing it.
// ===========================================================================

/// Real BFS-derived matrix counts for one workspace.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct R3a3RealMatrix {
    /// Routines emitting ≥1 inherited capability fact (genuine, distance ≥ 1).
    pub routines_with_inherited_facts: usize,
    /// Inherited facts whose GENUINE shortest witness distance is ≥ 2 hops.
    pub facts_with_more_than_1_hop_witness: usize,
    /// Inherited facts reached at their shortest distance by ≥2 DISTINCT first-hop
    /// edges (the equal-distance tie-breaker genuinely fired).
    pub equal_distance_ties: usize,
}

/// Compute the REAL (BFS-derived) anti-degenerate counts over a resolved
/// source-only workspace. Independent of `project_r3a3`'s cone path.
pub fn compute_r3a3_real_matrix(resolved: &L3Resolved) -> R3a3RealMatrix {
    let ws: &L3Workspace = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    let event_graph: EventGraph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    let nodes: Vec<String> = ws.routines.iter().map(|r| r.id.clone()).collect();
    let g = build_typed_edge_graph(&graph, &nodes);

    // Publisher events for the direct-fact key set.
    let mut publisher_events_by_routine: HashMap<String, Vec<&EventSymbol>> = HashMap::new();
    for evt in &event_graph.events {
        if let Some(pr) = &evt.publisher_routine_id {
            publisher_events_by_routine
                .entry(pr.clone())
                .or_default()
                .push(evt);
        }
    }

    // Per-routine SET of direct-fact dedup keys (inheritedFactKey).
    let empty_pub: Vec<&EventSymbol> = Vec::new();
    let mut direct_keys: HashMap<String, BTreeSet<String>> = HashMap::new();
    for r in &ws.routines {
        let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
        let (facts, _, _) = direct_facts_for_routine(r, pubs);
        let mut keys: BTreeSet<String> = BTreeSet::new();
        for f in &facts {
            keys.insert(inherited_fact_key(f));
        }
        if !keys.is_empty() {
            direct_keys.insert(r.id.clone(), keys);
        }
    }

    let empty_edges: Vec<TypedOutEdge> = Vec::new();
    let mut m = R3a3RealMatrix::default();

    for r in &ws.routines {
        let subject = &r.id;
        // Multi-source BFS over the typed-edge graph from `subject`. For each
        // reachable node we record its BFS distance from subject. We then, per
        // inheritedFactKey, find the MIN distance at which it first becomes
        // available (a callee at BFS distance d contributes its direct keys at
        // call-distance d+... — but the inheritedFactKey witness distance is the
        // shortest path on which the key appears). We compute, per key, the set of
        // FIRST-HOP edges that reach it at the genuine minimum total distance.

        // node -> shortest BFS distance from subject (excluding subject itself).
        let mut node_dist: HashMap<String, usize> = HashMap::new();
        // (firstHopKey, node) used to attribute first-hop edges.
        let mut queue: std::collections::VecDeque<(String, usize)> =
            std::collections::VecDeque::new();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        visited.insert(subject.clone());
        for e in g.outgoing.get(subject).unwrap_or(&empty_edges) {
            if visited.insert(e.to.clone()) {
                node_dist.insert(e.to.clone(), 0);
                queue.push_back((e.to.clone(), 0));
            }
        }
        while let Some((id, d)) = queue.pop_front() {
            for e in g.outgoing.get(&id).unwrap_or(&empty_edges) {
                if visited.insert(e.to.clone()) {
                    node_dist.insert(e.to.clone(), d + 1);
                    queue.push_back((e.to.clone(), d + 1));
                }
            }
        }

        // For each inheritedFactKey reachable through any callee, the witness
        // distance is `min over nodes carrying that key of (node_dist + 1)`. The
        // first-hop edges achieving that min are those whose target node lies on a
        // shortest path of that length. We approximate the first-hop SET by: for
        // each DIRECT successor edge `e`, run the key's reachable min-distance
        // *through that edge's subtree* — but the simplest sound tie test is: a
        // key is a tie iff ≥2 distinct first-hop edges each reach a key-carrying
        // node at the SAME minimal total distance.
        // Min witness distance per key:
        let mut key_min: BTreeMap<String, usize> = BTreeMap::new();
        for (node, nd) in &node_dist {
            if let Some(keys) = direct_keys.get(node) {
                for k in keys {
                    let cand = nd + 1;
                    key_min
                        .entry(k.clone())
                        .and_modify(|cur| {
                            if cand < *cur {
                                *cur = cand;
                            }
                        })
                        .or_insert(cand);
                }
            }
        }

        if key_min.is_empty() {
            continue;
        }
        m.routines_with_inherited_facts += 1;

        // Per first-hop edge: BFS within that edge's reachable subtree to find the
        // distance at which each key first appears VIA that first hop.
        // firstHopId -> (key -> minDistViaThisHop).
        let mut per_hop_key_dist: Vec<BTreeMap<String, usize>> = Vec::new();
        for fe in g.outgoing.get(subject).unwrap_or(&empty_edges) {
            let mut hop_node_dist: HashMap<String, usize> = HashMap::new();
            let mut hop_visited: BTreeSet<String> = BTreeSet::new();
            hop_visited.insert(subject.clone());
            let mut hq: std::collections::VecDeque<(String, usize)> =
                std::collections::VecDeque::new();
            if hop_visited.insert(fe.to.clone()) {
                hop_node_dist.insert(fe.to.clone(), 0);
                hq.push_back((fe.to.clone(), 0));
            }
            while let Some((id, d)) = hq.pop_front() {
                for e in g.outgoing.get(&id).unwrap_or(&empty_edges) {
                    if hop_visited.insert(e.to.clone()) {
                        hop_node_dist.insert(e.to.clone(), d + 1);
                        hq.push_back((e.to.clone(), d + 1));
                    }
                }
            }
            let mut hk: BTreeMap<String, usize> = BTreeMap::new();
            for (node, nd) in &hop_node_dist {
                if let Some(keys) = direct_keys.get(node) {
                    for k in keys {
                        let cand = nd + 1;
                        hk.entry(k.clone())
                            .and_modify(|cur| {
                                if cand < *cur {
                                    *cur = cand;
                                }
                            })
                            .or_insert(cand);
                    }
                }
            }
            per_hop_key_dist.push(hk);
        }

        for (k, &min_d) in &key_min {
            if min_d >= 2 {
                m.facts_with_more_than_1_hop_witness += 1;
            }
            // Tie: ≥2 distinct first-hop edges reach this key at the global min.
            let hop_count = per_hop_key_dist
                .iter()
                .filter(|hk| hk.get(k).copied() == Some(min_d))
                .count();
            if hop_count >= 2 {
                m.equal_distance_ties += 1;
            }
        }
    }

    m
}
