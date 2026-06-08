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
use crate::engine::l2::features::{PCallee, PExpressionInfo};
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
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
}

/// One normalized direct/inherited capability fact (internal form). `subject` is
/// kept for `repKey` tie-break parity but excluded from the projection.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Map an event kind to its CapabilityExtra event class (al-sem
/// `mapEventKindToClass`).
fn map_event_kind_to_class(event_kind: &str) -> &'static str {
    match event_kind {
        "IntegrationEvent" => "Integration",
        "BusinessEvent" => "Business",
        "InternalEvent" => "Internal",
        _ => "Trigger",
    }
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
    /// Declared type — needed by the member-expression (table-field) value-source
    /// branch the other IO/dispatch families use (Task 3); kept here so the
    /// classifier stays a faithful seam for that extension.
    #[allow(dead_code)]
    declared_type: String,
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
            let member = info.member.clone().or_else(|| info.value.clone());
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
            match variables.get(&name) {
                Some(v) if v.is_parameter => ValueSource::Parameter {
                    index: v.parameter_index,
                    var_name: name,
                },
                Some(_) => ValueSource::ConstantVar {
                    var_name: name,
                    initializer: Box::new(ValueSource::Unknown),
                },
                None => ValueSource::Expression,
            }
        }
        "unary_expression" => match &info.value {
            Some(v) => ValueSource::Literal { value: v.clone() },
            None => ValueSource::Expression,
        },
        _ => ValueSource::Expression,
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
fn direct_facts_for_routine(
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

    // Variable index for value-source classification.
    let mut variables: HashMap<String, VarInfo> = HashMap::new();
    for v in &routine.variables {
        variables
            .entry(v.name.to_lowercase())
            .or_insert_with(|| VarInfo {
                is_parameter: false,
                parameter_index: 0,
                declared_type: v.declared_type.clone(),
            });
    }
    for p in &routine.parameters {
        variables.insert(
            p.name.to_lowercase(),
            VarInfo {
                is_parameter: true,
                parameter_index: p.index,
                declared_type: p.type_text.clone(),
            },
        );
    }

    let mut facts: Vec<CapabilityFact> = Vec::new();
    let reasons: Vec<String> = Vec::new();

    // ── table family (al-sem table.ts) ─────────────────────────────────────
    for op in &routine.record_operations {
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

    // ── dispatch family (al-sem dispatch.ts) ───────────────────────────────
    for cs in &routine.call_sites {
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

fn value_source_to_json(vs: &ValueSource) -> serde_json::Value {
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

fn extra_to_json(e: &CapabilityExtra) -> serde_json::Value {
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
fn stable_routine_id(internal: &str, map: &HashMap<String, String>) -> String {
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

fn project_capability_fact(
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
fn capability_fact_sort_key(f: &PCapabilityFact) -> String {
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

    // Per-routine direct facts (full) + direct coverage + the dedup-keyed map.
    let mut direct_full: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
    let mut direct: RoutineDirectFacts = HashMap::new();
    let mut cov: RoutineDirectCoverage = HashMap::new();
    let mut routine_ids: BTreeSet<String> = BTreeSet::new();
    let empty_pub: Vec<&EventSymbol> = Vec::new();

    for r in &ws.routines {
        routine_ids.insert(r.id.clone());
        let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
        let (facts, status, reasons) = direct_facts_for_routine(r, pubs);

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
        cov.insert(
            r.id.clone(),
            DirectCoverage {
                direct_status: status,
                reasons,
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
