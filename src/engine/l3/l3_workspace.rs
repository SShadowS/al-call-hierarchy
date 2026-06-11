//! L3 workspace assembly — the WORKSPACE-level model the L3 resolver needs.
//!
//! Where L2 (`l2_workspace::project_workspace`) processed per-file/per-object and
//! emitted a flat allowlisted projection, L3 needs ALL objects + tables +
//! routines across the whole workspace assembled together, in al-sem's EXACT
//! deterministic ingestion order: POSIX-path-sorted files → per-file document
//! order (the same order R0/R1 discovery produces). This order is LOAD-BEARING —
//! the symbol table's name/number collision resolution is LAST-wins, the
//! lexical-scope fallback is LAST-wins, and `merge_extension_fields` is FIRST-wins,
//! all keyed off this iteration order.
//!
//! This module also drives resolution: `build_symbol_table → resolve_record_types
//! → merge_extension_fields` (al-sem's first three resolve sub-steps; calls /
//! events / coverage are LATER gates and OUT of R2a). Record vars / ops get their
//! resolved internal `tableId`, projected to a StableTableId by the test/dump.
//!
//! Object / routine features reuse the L2 body walk verbatim
//! (`project_routine_features`) so record vars / ops / variables match L2
//! byte-for-byte; L3 only adds the table/field index + the cross-file resolution.

use super::extension_fields::merge_extension_fields;
use super::record_types::resolve_routine_record_types;
use super::symbol_table::SymbolTable;
use crate::engine::ids::{encode_object_id, to_stable_object_id, to_stable_routine_id_from_parts};
use crate::engine::l2::node_util::{named_children, node_text, strip_quotes, Utf16Cols};
use crate::engine::l2::scope;
use crate::engine::l2::{
    extract_object_number, project_routine_features, routine_normalized_signature_hash, IdentityCtx,
};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// L3 model types — workspace-level, in-memory (NOT the serde projection shape).
// ---------------------------------------------------------------------------

/// A workspace object (the L3-relevant subset of al-sem's `ObjectDecl`).
#[derive(Debug, Clone)]
pub struct L3Object {
    /// Internal object id: `${appGuid}/${objectType}/${objectNumber}`.
    pub id: String,
    pub app_guid: String,
    pub object_type: String,
    pub object_number: i64,
    pub name: String,
    /// Page / PageExtension `SourceTable` (unquoted), else None.
    pub source_table_name: Option<String>,
    /// TableExtension / PageExtension `extends` target (unquoted), else None.
    pub extends_target_name: Option<String>,
    /// Implemented interfaces (Codeunit / Enum / Interface): `Some([])` known-none,
    /// `Some([...])` listed, `None` unknown. Other object types: `None`.
    pub implements_interfaces: Option<Vec<String>>,
    /// Object `Subtype` property (Codeunit only; e.g. "Install" / "Upgrade" /
    /// "Test"), else `None`. Additive L2→L3 forward — L3Object is NOT
    /// Serialize-derived into any gate surface (R0–R3 goldens are field-allowlisted
    /// projections), so adding this never touches a golden. Populated at L3 assembly
    /// (native path) from the `Subtype` property; dep objects forward the ABI
    /// projection's `object_subtype` (it DOES expose it for Codeunits — native+ABI
    /// agree on shape). Consumed by d46 to classify lifecycle objects.
    pub object_subtype: Option<String>,
    /// Object `PageType` property (Page / PageExtension only; e.g. "API" /
    /// "Card" / "List"), else `None`. Additive L2→L3 forward — L3Object is NOT
    /// Serialize-derived into any gate surface (R0–R3 goldens are
    /// field-allowlisted projections), so adding this never touches a golden.
    /// Populated at L3 assembly (native path) from the `PageType` property; dep
    /// objects use `None` (the ABI projection does not expose it — no API-page
    /// fixture in the R4-F corpus exercises it). Consumed by R4-F
    /// `root_classification::kinds_for` to classify `api-page` roots.
    pub page_type: Option<String>,
    /// Object `InherentCommitBehavior` property (Codeunit / Table /
    /// TableExtension only). Canonical lower-case member: "ignore" | "error" |
    /// "allow". `None` when absent or an unrecognised value. Additive L2→L3
    /// forward — L3Object is NOT Serialize-derived into any gate surface, so
    /// adding this never touches a golden. Populated at L3 assembly (native
    /// path) from the `InherentCommitBehavior` property; dep objects forward the
    /// ABI projection's `inherent_commit_behavior` (it carries the same canonical
    /// lower-case form). Consumed by R4-F `return_summary` to merge the
    /// object-level commit behavior into each routine's `commitBehavior`.
    pub inherent_commit_behavior: Option<String>,
    /// Page / PageExtension `SourceTable` temporary flag — `Some(true)` when the
    /// SourceTable object is marked `TableType = Temporary`, `Some(false)` when
    /// confirmed non-temporary, `None` when not yet resolved. Additive — L3Object
    /// is NOT Serialize-derived into any gate surface, so this never touches a
    /// golden. Populated by later tasks (Task 5).
    pub source_table_temporary: Option<bool>,
}

/// A workspace field (the L3-relevant subset of al-sem's `Field`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L3Field {
    pub id: String,
    pub physical_table_id: String,
    pub declaring_object_id: String,
    pub declaring_app_id: String,
    pub field_number: i64,
    pub name: String,
    pub field_class: String,
    pub data_type: String,
    pub is_blob_like: bool,
}

/// A workspace table key (the L3-relevant subset of al-sem's `Key`). Only the
/// fields the cli-b snapshot `deriveSchema` reads are kept (`id` + resolved
/// member field-ids). Additive — `L3Table` is NOT serialized into any R0–R3
/// golden surface, so adding keys never moves an existing golden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L3Key {
    /// Internal key id: `${tableId}/key/${index}` (mirrors al-sem `encodeKeyId`).
    pub id: String,
    /// Resolved member field internal ids (`${tableId}/${fieldNumber}`), in
    /// declaration order. A key field not found in this object is silently
    /// skipped (mirrors al-sem's `fieldsByName` resolution).
    pub fields: Vec<String>,
}

/// A workspace table (the L3-relevant subset of al-sem's `Table`). Both `Table`
/// and `TableExtension` declarations produce one of these (matching al-sem's
/// `index.tables`).
#[derive(Debug, Clone)]
pub struct L3Table {
    /// Internal table id: `${appGuid}/table/${tableNumber}`.
    pub id: String,
    pub app_guid: String,
    pub table_number: i64,
    pub name: String,
    pub fields: Vec<L3Field>,
    /// Table keys (cli-b snapshot `deriveSchema` reads these). Additive.
    pub keys: Vec<L3Key>,
    /// True when the table is declared with `TableType = Temporary`. Additive —
    /// L3Table is NOT Serialize-derived into any gate surface, so this never
    /// touches a golden. Populated by later tasks (Task 4).
    pub is_temporary: bool,
}

/// A record variable with its (post-resolve) resolved internal table id.
#[derive(Debug, Clone)]
pub struct L3RecordVariable {
    pub id: String,
    pub name: String,
    /// Declared table name (unquoted), or None for a non-record / unparsed type.
    pub table_name: Option<String>,
    /// Resolved internal TableId, set by `resolve_record_types`. None = unresolved.
    pub table_id: Option<String>,
    /// True when this is a parameter record variable (from L2 body walk).
    /// Required by the R3a-2 summary engine to derive RecordRoleSummary per
    /// record parameter (mirrors al-sem `recVar.isParameter`).
    pub is_parameter: bool,
    /// The 0-based parameter index when `is_parameter` is true.
    /// Required by the R3a-2 summary engine (`recVar.parameterIndex`).
    pub parameter_index: Option<u32>,
    /// Temp-state of this record variable (from the L2 body walk). al-sem d3 reads
    /// `recVar.tempState` to skip temporary records (SetLoadFields has no SQL
    /// benefit for in-memory temp records). Additive L2→L3 forward — the L3
    /// record-type projection is field-allowlisted, so this never reaches an
    /// R0–R3 golden. Forwarded verbatim from `PRecordVariable.temp_state`.
    pub temp_state: crate::engine::l2::features::PTempState,
    /// Variable scope: `"local"` | `"parameter"` | `"global"`. `None` when not
    /// yet populated. Additive — forwarded from `PRecordVariable.scope`;
    /// the L3 record-type projection is field-allowlisted, so this never reaches
    /// a golden. Populated by later tasks.
    pub scope: Option<String>,
}

impl L3RecordVariable {
    /// `recVar.tempState.kind === "known" ? recVar.tempState.value : None`.
    /// Returns `Some(value)` only when the temp state is concretely known.
    pub fn temp_state_known_value(&self) -> Option<bool> {
        if self.temp_state.kind == "known" {
            self.temp_state.value
        } else {
            None
        }
    }
}

/// A record operation with its (post-resolve) resolved internal table id.
#[derive(Debug, Clone)]
pub struct L3RecordOperation {
    pub id: String,
    pub op: String,
    pub record_variable_name: String,
    pub record_variable_id: Option<String>,
    pub table_id: Option<String>,
    /// Temp-state of this operation (from L2 body walk). Required by the
    /// R3a-2 summary engine to derive DbEffect.tempState for base summaries.
    pub temp_state: Option<crate::engine::l2::features::PTempState>,
    /// Field arguments for ops like Validate (from L2 body walk). Required
    /// by the R3a-2 summary engine for RecordRoleSummary.writesFields.
    pub field_arguments: Option<Vec<String>>,
    /// Source anchor (from L2 body walk). Required by the R3a-2 branch-aware
    /// CFG walker to interleave record ops with field accesses by source
    /// position inside a block (mirrors al-sem `op.sourceAnchor.range`). L2 data
    /// that the L3 record-type projection drops, forwarded here for L4 only.
    pub source_anchor: crate::engine::l2::features::PAnchor,
    /// The enclosing-loop id stack (from L2 body walk). L2 data that the L3
    /// record-type projection drops, forwarded here for L5 detectors (d4 reads
    /// `op.loopStack.includes(loop.id)`). Additive — the L3 projections are
    /// field-allowlisted, so this never reaches an R0–R3 golden.
    pub loop_stack: Vec<String>,
    /// Structured field-argument classification (from L2 body walk). L2 data that
    /// the L3 record-type projection drops, forwarded here for L5 detectors (d4
    /// reads `op.fieldArgumentInfos[0]` for the literal-key test). Additive.
    pub field_argument_infos: Option<Vec<crate::engine::l2::features::PExpressionInfo>>,
}

/// A lexical variable (params → locals → globals) carrying its declared type, for
/// the record-op lexical-scope fallback.
#[derive(Debug, Clone)]
pub struct L3Variable {
    pub name: String,
    pub declared_type: String,
    /// True when this variable is a routine parameter. Required by the R3a-3 L4
    /// value-source classifier (`classifyIdentifier`: a parameter resolves to a
    /// `parameter` ValueSource, a local to its initializer / `constant-var`).
    pub is_parameter: bool,
    /// 0-based parameter index when `is_parameter`.
    pub parameter_index: Option<u32>,
    /// The L2-captured one-hop initializer (`VariableSymbol.initializer`) as raw
    /// ValueSource JSON. Required by the R3a-3 value-source classifier so local
    /// dispatch / IO targets resolve to their literal/enum source (e.g.
    /// `CodeunitId := 50100; Codeunit.Run(CodeunitId)` → literal 50100).
    pub initializer: Option<serde_json::Value>,
}

/// A routine parameter (the L3-relevant subset of al-sem's `ParameterSymbol`) —
/// drives arity matching, `calleeParameterIsVar` upgrades, and overload arg-type
/// disambiguation (`typeText`).
#[derive(Debug, Clone)]
pub struct L3Parameter {
    /// Positional index (0-based) — the EventSymbol parameter shape.
    pub index: u32,
    pub name: String,
    pub type_text: String,
    pub is_var: bool,
    /// True when the declared type is a `Record` (drives the event param shape).
    pub is_record: bool,
    /// Record table name (unquoted), when `is_record`.
    pub table_name: Option<String>,
}

/// A workspace routine (the L3-relevant subset).
#[derive(Debug, Clone)]
pub struct L3Routine {
    /// Internal routine id: `${modelInstanceId}/${canonicalRoutineKeyHash}`.
    pub id: String,
    /// StableRoutineId: `${stableObjectId}#${normalizedSignatureHash}` — the
    /// modelInstanceId-independent key the L3 record-type projection emits.
    pub stable_routine_id: String,
    /// Owning object's internal id.
    pub object_id: String,
    /// Owning object's type (`Codeunit` / `Page` / `Table` / …) for the projection.
    pub object_type: String,
    pub name: String,
    /// Routine kind (`procedure` / `trigger` / `event-publisher` /
    /// `event-subscriber`) — drives the event-graph publisher/subscriber passes.
    pub kind: String,
    /// Structured attributes (the grammar-derived AttributeInfo shape) — the event
    /// graph reads `[IntegrationEvent]`/`[BusinessEvent]`/`[EventSubscriber]` args.
    pub attributes_parsed: Vec<super::al_attributes::AttributeInfo>,
    /// Owning object's app guid — the EventEdge `subscriberAppId`.
    pub app_guid: String,
    /// Owning object's number — for the publisher's `publisherObjectId`.
    pub object_number: i64,
    /// The return-type-aware normalized signature hash — the EventSymbol
    /// `signatureHash` for REAL publisher routines.
    pub normalized_signature_hash: String,
    /// L2 `bodyAvailable` — the routine has a `code_block` body (routine-indexer.ts).
    /// The L3 coverage (R2d) counts these. Set from the SAME `find_code_block`
    /// the L2 projection uses so the flag cannot drift.
    pub body_available: bool,
    /// L2 `parseIncomplete` — the routine's subtree has a tree-sitter ERROR node.
    /// R2d projects parse-incomplete routines' StableRoutineIds.
    pub parse_incomplete: bool,
    pub record_variables: Vec<L3RecordVariable>,
    pub record_operations: Vec<L3RecordOperation>,
    /// Field accesses (from L2 body walk). Required by the R3a-2 summary
    /// engine to derive RecordRoleSummary.readsFields per record parameter.
    pub field_accesses: Vec<crate::engine::l2::features::PFieldAccess>,
    pub variables: Vec<L3Variable>,
    /// Declared parameters (in order) — drives arity + var-ness + arg-type
    /// disambiguation. Empty for trigger routines with no parameter list.
    pub parameters: Vec<L3Parameter>,
    /// Access modifier from the L2 projection (`local`/`internal`/`protected`; None
    /// = default/public). Additive field — L3Routine is NOT Serialize-derived into
    /// any gate surface (R0–R3 goldens are field-allowlisted projections), so adding
    /// this never touches a golden. Populated by the native assembly path from
    /// `classify_access_modifier`; dep routines use `None` (ABI does not expose it).
    /// Consumed by d32 (scope gate: `local` only).
    pub access_modifier: Option<String>,
    /// Declared return type text (`type_specification` text), if any — used by
    /// `inferCallExprReturnType` for overload arg-type disambiguation.
    pub return_type: Option<String>,
    /// The routine's call sites (L2 body-walk output), the resolver input.
    pub call_sites: Vec<crate::engine::l2::features::PCallSite>,
    /// The routine's operation sites (L2 body-walk output). Required by the R3a-3
    /// L4 capability extraction (commit family reads `kind === "commit"`, error
    /// family reads `kind === "error-call"`), and the unreachable-exclusion pass
    /// (sites with `controlContext === "unreachable"` are dropped before family
    /// dispatch — mirrors al-sem `extractCapabilities`).
    pub operation_sites: Vec<crate::engine::l2::features::POperationSite>,
    /// The CFN statement-tree skeleton (L2 body-walk output). Required by the
    /// R3a-2 branch-aware CFG walker (`walkCFG` port) to join role state-sets at
    /// if/case/loop. `None` for opaque / TryFunction / bodyless routines (the
    /// walker then falls back to the straight-line pass, mirroring al-sem).
    pub statement_tree: Option<crate::engine::l2::features::PCFNNode>,
    /// The routine's loops (L2 body-walk output). L2 data that the L3
    /// record-type projection drops, forwarded here for L5 detectors (d4 reads
    /// `routine.features.loops`). Additive — never reaches an R0–R3 golden.
    pub loops: Vec<crate::engine::l2::features::PLoop>,
    /// The routine's OWN declaration anchor (the procedure / trigger_declaration
    /// node range, with `syntax_kind` = "procedure" / "trigger_declaration").
    /// al-sem `routine-indexer.ts:419` builds this as the routine's `sourceAnchor`.
    /// Captured during L2/L3 assembly where the routine NODE is available. Read by
    /// d19 (primaryLocation + evidence) and d29 (first evidence step). Additive —
    /// the L3 record-type projection is field-allowlisted so this never reaches an
    /// R0–R3 golden.
    pub source_anchor: crate::engine::l2::features::PAnchor,
    /// Lowercased / sorted / deduped identifier references in the routine body
    /// (L2 features `identifierReferences`). Read by d19 to test parameter use.
    /// Additive — forwarded verbatim from L2.
    pub identifier_references: Vec<String>,
    /// Unreachable-after-exit statements recorded during the L2 body DFS
    /// (`features.unreachableStatements`). Read by d20. Additive — forwarded verbatim.
    pub unreachable_statements: Vec<crate::engine::l2::features::PUnreachableStatement>,
    /// Whether the routine body contains any branching (`features.hasBranching`).
    /// Read by d43's `classify_subscriber` / `publisher_branch_facts`. Additive —
    /// forwarded verbatim from L2; dep (bodyless) routines default `false`.
    pub has_branching: bool,
    /// Variable assignments (`features.varAssignments`) — `lhsName` + optional
    /// `rhsLiteralValue`. Read by d43 to detect `IsHandled := true` setters.
    /// Additive — forwarded verbatim from L2.
    pub var_assignments: Vec<crate::engine::l2::features::PVarAssignment>,
    /// Condition references (`features.conditionReferences`) — identifiers used in
    /// guard positions, with their reference anchors. Read by `enumerate_dispatch_sites`
    /// (d43) to find post-call IsHandled guards. Additive — forwarded verbatim from L2.
    pub condition_references: Vec<crate::engine::l2::features::PConditionReference>,
    /// Field/control/action/dataitem member name for a member-trigger routine — the
    /// unescaped logical identifier (inner `""` collapsed to `"`) of the enclosing
    /// member wrapper (field_declaration / page_field / action_declaration /
    /// report_dataitem / query_dataitem). `None` for procedures and object-level
    /// triggers (OnRun / OnOpenPage). Additive — `L3Routine` is NOT `Serialize`-derived
    /// (it has only `#[derive(Debug, Clone)]`), so this never reaches an R0–R3 golden.
    /// (RE-3 / RE-4)
    pub enclosing_member: Option<String>,
    /// StableObjectId of the object that DECLARES this trigger (the object decl in
    /// scope at assembly — the EXTENSION object for an extension-declared trigger).
    /// Honest metadata; the AL CPU-profile frame carries no extension identity, so this
    /// is profile-UNJOINABLE for the multi-extension collision (RE-5). `None` for
    /// non-member routines. Additive — never reaches an R0–R3 golden.
    pub originating_object: Option<String>,
    /// Source range of the member WRAPPER node (field_declaration / page_field /
    /// action_declaration / report_dataitem / query_dataitem) — the boundary the
    /// finding-side position discriminator (E3) matches a finding's primaryLocation
    /// against. `None` for non-member routines. Additive — never reaches an R0–R3
    /// golden. (RE-2)
    pub enclosing_member_range: Option<crate::engine::l2::features::PAnchor>,
}

/// The assembled workspace L3 model (pre-resolve until `resolve` runs).
#[derive(Debug, Clone)]
pub struct L3Workspace {
    pub objects: Vec<L3Object>,
    pub tables: Vec<L3Table>,
    pub routines: Vec<L3Routine>,
}

// ---------------------------------------------------------------------------
// Object metadata extraction (object-indexer.ts parity).
// ---------------------------------------------------------------------------

/// `extractObjectName` — first quoted_identifier (stripped) or identifier, else "".
fn extract_object_name(decl: Node, source: &str) -> String {
    for child in named_children(decl) {
        match child.kind() {
            "quoted_identifier" => return strip_quotes(node_text(child, source)).to_string(),
            "identifier" => return node_text(child, source).to_string(),
            _ => {}
        }
    }
    String::new()
}

/// `readObjectProperty` — first DIRECT `property` child whose `name` field matches
/// (case-insensitive); returns the raw `value` field text. Never descends.
fn read_object_property(decl: Node, property_name: &str, source: &str) -> Option<String> {
    let want = property_name.to_lowercase();
    for child in named_children(decl) {
        if child.kind() != "property" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        if node_text(name_node, source).to_lowercase() != want {
            continue;
        }
        return child
            .child_by_field_name("value")
            .map(|v| node_text(v, source).to_string());
    }
    None
}

/// `extractExtendsTargetName` — first identifier / quoted_identifier (stripped)
/// after the `extends_keyword` child.
fn extract_extends_target_name(decl: Node, source: &str) -> Option<String> {
    let mut saw_extends = false;
    for child in named_children(decl) {
        if child.kind() == "extends_keyword" {
            saw_extends = true;
            continue;
        }
        if !saw_extends {
            continue;
        }
        match child.kind() {
            "quoted_identifier" => return Some(strip_quotes(node_text(child, source)).to_string()),
            "identifier" => return Some(node_text(child, source).to_string()),
            _ => {}
        }
    }
    None
}

/// `extractImplementsInterfaces` — names after the `implements` keyword (unquoted),
/// in document order. Returns `Some([])` when the object type can carry the clause
/// but none are present. Mirrors object-indexer.ts (Codeunit / Enum / Interface).
fn extract_implements_interfaces(decl: Node, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut saw_implements = false;
    for child in named_children(decl) {
        let kind = child.kind();
        if kind == "implements_keyword" {
            saw_implements = true;
            continue;
        }
        if !saw_implements {
            // Some grammars wrap the list in an `implements_clause` node — descend.
            if kind == "implements_clause" {
                for sub in named_children(child) {
                    match sub.kind() {
                        "quoted_identifier" => {
                            out.push(strip_quotes(node_text(sub, source)).to_string())
                        }
                        "identifier" => out.push(node_text(sub, source).to_string()),
                        _ => {}
                    }
                }
            }
            continue;
        }
        match kind {
            "quoted_identifier" => out.push(strip_quotes(node_text(child, source)).to_string()),
            "identifier" => out.push(node_text(child, source).to_string()),
            // Stop at the body brace / first non-name child.
            "object_body" | "code_block" => break,
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Table / field extraction (object-indexer.ts `indexTable` / `classifyField`).
// ---------------------------------------------------------------------------

const BLOB_LIKE: &[&str] = &["blob", "media", "mediaset"];

/// `classifyField` — (dataType, fieldClass, isBlobLike).
fn classify_field(field_node: Node, source: &str) -> (String, String, bool) {
    let mut data_type = String::new();
    for child in named_children(field_node) {
        if child.kind() == "type_specification" {
            data_type = node_text(child, source).to_string();
            break;
        }
    }
    let mut field_class = "Normal".to_string();
    for child in named_children(field_node) {
        if child.kind() != "property" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        if node_text(name_node, source).to_lowercase() != "fieldclass" {
            continue;
        }
        let value = child
            .child_by_field_name("value")
            .map(|v| node_text(v, source).to_lowercase())
            .unwrap_or_default();
        if value.contains("flowfield") {
            field_class = "FlowField".to_string();
        } else if value.contains("flowfilter") {
            field_class = "FlowFilter".to_string();
        }
    }
    let is_blob_like = BLOB_LIKE.contains(&data_type.to_lowercase().as_str());
    (data_type, field_class, is_blob_like)
}

/// `indexTable` — build an `L3Table` (fields only; keys are OUT for R2a) from a
/// table / tableextension declaration.
fn index_table(
    decl: Node,
    object_id: &str,
    app_guid: &str,
    table_number: i64,
    table_name: &str,
    source: &str,
) -> L3Table {
    let table_id = format!("{app_guid}/table/{table_number}");
    let mut fields = Vec::new();

    // Collect `field_declaration` + `key_declaration` nodes anywhere under the
    // declaration (prune at match — don't recurse into a matched node's own
    // children). Document order. Mirrors al-sem `indexTable`'s single DFS.
    let mut field_nodes: Vec<Node> = Vec::new();
    let mut key_nodes: Vec<Node> = Vec::new();
    let mut stack = vec![decl];
    let mut buffer: Vec<Node> = Vec::new();
    let mut key_buffer: Vec<Node> = Vec::new();
    while let Some(node) = stack.pop() {
        if node.kind() == "field_declaration" {
            buffer.push(node);
            continue;
        }
        if node.kind() == "key_declaration" {
            key_buffer.push(node);
            continue;
        }
        // Push children reversed so the (reversed) collection reads document order.
        let children = named_children(node);
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    // `stack.pop()` + reverse-push yields document order already; collect.
    field_nodes.extend(buffer);
    key_nodes.extend(key_buffer);

    for field_node in &field_nodes {
        let mut field_number = 0i64;
        let mut field_name = String::new();
        let mut name_found = false;
        for child in named_children(*field_node) {
            if field_number == 0 && child.kind() == "integer" {
                field_number = node_text(child, source).trim().parse::<i64>().unwrap_or(0);
                continue;
            }
            if !name_found && field_number != 0 {
                match child.kind() {
                    "quoted_identifier" => {
                        field_name = strip_quotes(node_text(child, source)).to_string();
                        name_found = true;
                    }
                    "identifier" => {
                        field_name = node_text(child, source).to_string();
                        name_found = true;
                    }
                    _ => {}
                }
            }
        }
        let (data_type, field_class, is_blob_like) = classify_field(*field_node, source);
        fields.push(L3Field {
            id: format!("{table_id}/{field_number}"),
            physical_table_id: table_id.clone(),
            declaring_object_id: object_id.to_string(),
            declaring_app_id: app_guid.to_string(),
            field_number,
            name: field_name,
            field_class,
            data_type,
            is_blob_like,
        });
    }

    // Resolve key member fields by (lowercased) name → field id, mirroring
    // al-sem `indexTable`'s `fieldsByName` resolution. A key field not present in
    // this object is silently skipped.
    let fields_by_name: std::collections::HashMap<String, String> = fields
        .iter()
        .map(|f| (f.name.to_lowercase(), f.id.clone()))
        .collect();
    let mut keys: Vec<L3Key> = Vec::new();
    for (index, key_node) in key_nodes.iter().enumerate() {
        let mut key_field_ids: Vec<String> = Vec::new();
        // The member list lives in a `field_list` child; its named children are
        // identifier / quoted_identifier name references.
        if let Some(field_list) = named_children(*key_node)
            .into_iter()
            .find(|c| c.kind() == "field_list")
        {
            for child in named_children(field_list) {
                let raw = node_text(child, source);
                let name = strip_quotes(raw).to_lowercase();
                if let Some(fid) = fields_by_name.get(&name) {
                    key_field_ids.push(fid.clone());
                }
            }
        }
        keys.push(L3Key {
            id: format!("{table_id}/key/{index}"),
            fields: key_field_ids,
        });
    }

    // Part A (Task 4 / G3): native `TableType = Temporary` capture. The ONLY
    // allowed temp signal at this layer — a structural property read. EXACT
    // case-insensitive match (trim + lowercase + `== "temporary"`); never
    // `.contains()` / string-sniffing. A missing / other value → false
    // (conservative; the engine never throws).
    let is_temporary = read_object_property(decl, "TableType", source)
        .map(|v| v.trim().to_lowercase() == "temporary")
        .unwrap_or(false);

    L3Table {
        id: table_id,
        app_guid: app_guid.to_string(),
        table_number,
        name: table_name.to_string(),
        fields,
        keys,
        is_temporary,
    }
}

/// Build a `PAnchor` from a node + the file's UTF-16 column index, mirroring the
/// L2 body-walk `Ctx::anchor`. Used to capture the routine's OWN declaration
/// anchor (`syntax_kind` = node.kind() = "procedure" / "trigger_declaration"),
/// matching al-sem `routine-indexer.ts:419`'s `sourceAnchor`.
fn anchor_from_node(
    node: Node,
    source_unit_id: &str,
    cols: &Utf16Cols,
) -> crate::engine::l2::features::PAnchor {
    let sp = node.start_position();
    let ep = node.end_position();
    crate::engine::l2::features::PAnchor {
        source_unit_id: source_unit_id.to_string(),
        start_line: sp.row as u32,
        start_column: cols.col(sp.row, sp.column),
        end_line: ep.row as u32,
        end_column: cols.col(ep.row, ep.column),
        syntax_kind: node.kind().to_string(),
    }
}

/// `collectDescendants(prune-at-match)` for procedure / trigger_declaration.
///
/// Returns `(parent, routine)` pairs: the routine node plus its immediate
/// `node.parent()` at the DFS match point (RE-7), captured WITHOUT restructuring the
/// stack / push order so the traversal — and therefore the routine set + order — is
/// byte-for-byte unchanged from the pre-E1 `Vec<Node>` form. The parent enables the
/// member-trigger enclosing-member derivation (`enclosing_member_of`); object-level
/// triggers / procedures carry a non-member-bearing parent and resolve to `None`.
fn collect_routine_nodes(decl: Node) -> Vec<(Option<Node>, Node)> {
    let mut out = Vec::new();
    let mut stack = vec![decl];
    while let Some(node) = stack.pop() {
        if node.kind() == "procedure" || node.kind() == "trigger_declaration" {
            out.push((node.parent(), node));
            continue;
        }
        for child in named_children(node).into_iter().rev() {
            stack.push(child);
        }
    }
    out
}

/// Unescape an AL identifier's logical name: a quoted AL identifier escapes an inner
/// double-quote by doubling it (`""`), so the logical name collapses each `""` back to
/// a single `"`. Called AFTER `strip_quotes` (which only trims the boundary quotes), so
/// the input here is the inner text. Matches the profiler's display form (RE-4).
fn unescape_al_identifier(inner: &str) -> String {
    inner.replace("\"\"", "\"")
}

/// Returns `(unescaped-logical-member-name, wrapper_node)` when `parent` is a
/// member-bearing wrapper, else `None`.
///
/// RULE (RE-3): a member-bearing parent is any immediate parent that is NOT the object
/// declaration AND exposes a `name` field — `field_declaration` (whose first named child
/// is the integer field NUMBER, so the name MUST come via `child_by_field_name("name")`,
/// not "first child"), `page_field`, `action_declaration`, `report_dataitem`,
/// `query_dataitem`. Object declarations expose `object_name` (not `name`) so they are
/// excluded by construction; true object-level triggers (`OnRun` / `OnOpenPage`) have a
/// non-member parent → `None`. `actionref_declaration` uses `promoted_name` (no `name`)
/// → `None`. The name is `strip_quotes`'d then `unescape_al_identifier`'d (RE-4).
fn enclosing_member_of<'a>(parent: Option<Node<'a>>, source: &str) -> Option<(String, Node<'a>)> {
    let p = parent?;
    let name_node = p.child_by_field_name("name")?;
    let raw = node_text(name_node, source);
    Some((unescape_al_identifier(strip_quotes(raw)), p))
}

// ---------------------------------------------------------------------------
// Per-file assembly.
// ---------------------------------------------------------------------------

const MODEL_INSTANCE_ID_DEFAULT: &str = "r0";

#[allow(clippy::too_many_arguments)]
fn project_file(
    root: Node,
    source: &str,
    app_guid: &str,
    model_instance_id: &str,
    source_unit_id: &str,
    cols: &Utf16Cols,
    workspace: &mut L3Workspace,
) {
    for decl in named_children(root) {
        let Some(object_type) = scope::object_type_for(decl.kind()) else {
            continue;
        };
        let object_number = extract_object_number(decl, source);
        let name = extract_object_name(decl, source);
        let object_id = encode_object_id(app_guid, object_type, object_number);

        // Object metadata (object-indexer.ts parity).
        let source_table_name = if object_type == "Page" || object_type == "PageExtension" {
            read_object_property(decl, "SourceTable", source).map(|s| strip_quotes(&s).to_string())
        } else {
            None
        };
        let extends_target_name =
            if object_type == "TableExtension" || object_type == "PageExtension" {
                extract_extends_target_name(decl, source)
            } else {
                None
            };
        let implements_interfaces =
            if object_type == "Codeunit" || object_type == "Enum" || object_type == "Interface" {
                Some(extract_implements_interfaces(decl, source))
            } else {
                None
            };
        // Object `Subtype` — Codeunit only (object-indexer.ts parity / L2
        // `extract_object_metadata`). d46 reads this to classify Install/Upgrade
        // lifecycle codeunits.
        let object_subtype = if object_type == "Codeunit" {
            read_object_property(decl, "Subtype", source)
        } else {
            None
        };
        // Object `PageType` — Page / PageExtension only (object-indexer.ts
        // `pageType` parity). R4-F `root_classification` reads this to classify
        // `api-page` roots (PageType == "API", case-insensitive).
        let page_type = if object_type == "Page" || object_type == "PageExtension" {
            read_object_property(decl, "PageType", source)
        } else {
            None
        };
        // Object `InherentCommitBehavior` — Codeunit / Table / TableExtension only
        // (object-indexer.ts parity). The raw value is a qualified_enum_value like
        // "InherentCommitBehavior::Ignore"; extract the member after "::", then
        // lower-case. Unrecognised values → None (conservative).
        let inherent_commit_behavior = if object_type == "Codeunit"
            || object_type == "Table"
            || object_type == "TableExtension"
        {
            read_object_property(decl, "InherentCommitBehavior", source).and_then(|raw| {
                let sep = raw.rfind("::").map(|i| i + 2).unwrap_or(0);
                let member = raw[sep..].to_lowercase();
                match member.as_str() {
                    "ignore" => Some("ignore".to_string()),
                    "error" => Some("error".to_string()),
                    "allow" => Some("allow".to_string()),
                    _ => None,
                }
            })
        } else {
            None
        };
        // Object `SourceTableTemporary` — Page / PageExtension only (Task 5 / G4).
        // Structural boolean property: `SourceTableTemporary = true;` means the
        // page's implicit `Rec` and `xRec` are always temporary. The value node is
        // an `identifier` (`true` / `false`); exact case-insensitive match against
        // "true" — anything else (missing / "false" / unrecognised) → `Some(false)`
        // when the property is present but non-"true", or `None` when absent.
        // EXACT match — no string-sniffing. Engine never throws; missing → None.
        let source_table_temporary = if object_type == "Page" || object_type == "PageExtension" {
            read_object_property(decl, "SourceTableTemporary", source)
                .map(|v| v.trim().to_lowercase() == "true")
        } else {
            None
        };

        workspace.objects.push(L3Object {
            id: object_id.clone(),
            app_guid: app_guid.to_string(),
            object_type: object_type.to_string(),
            object_number,
            name: name.clone(),
            source_table_name: source_table_name.clone(),
            extends_target_name,
            implements_interfaces,
            object_subtype,
            page_type,
            inherent_commit_behavior,
            source_table_temporary,
        });

        if object_type == "Table" || object_type == "TableExtension" {
            workspace.tables.push(index_table(
                decl,
                &object_id,
                app_guid,
                object_number,
                &name,
                source,
            ));
        }

        // Object globals + per-routine features (reuse the L2 body walk verbatim).
        let object_globals = scope::extract_object_globals(decl, source_unit_id, source);
        // Task 3 (temp-state): object-global RECORD vars carry the temp signal the
        // L2 body walk never saw (it only knew params + locals). Capture them once
        // per object and promote (below) into each routine's `record_variables`,
        // honoring AL shadowing — a routine's OWN param/local of the same name wins.
        let object_global_record_vars =
            scope::extract_object_global_record_vars(decl, &object_id, source);
        let routine_nodes = collect_routine_nodes(decl);
        let mut object_procedure_names = std::collections::HashSet::new();
        for (_parent, n) in &routine_nodes {
            if let Some(nm) = n.child_by_field_name("name") {
                object_procedure_names.insert(strip_quotes(node_text(nm, source)).to_lowercase());
            }
        }
        let id_ctx = IdentityCtx {
            app_guid,
            model_instance_id,
            source_unit_id,
        };

        for (member_parent, routine) in routine_nodes {
            let Some(nm) = routine.child_by_field_name("name") else {
                continue;
            };
            let rname = strip_quotes(node_text(nm, source)).to_string();
            if rname.is_empty() {
                continue;
            }

            let Some((routine_id, mut features)) = project_routine_features(
                decl,
                routine,
                object_type,
                object_number,
                source_table_name.as_deref(),
                &object_procedure_names,
                &object_globals,
                &id_ctx,
                source,
                cols,
            ) else {
                continue;
            };

            // R1b control-context lattice (the SAME pass `aldump --l2` applies):
            // populate `controlContext` on each callsite/operation site, including the
            // error-call source-range post-pass. Required by the R3a-3 L4 capability
            // extraction's UNREACHABLE EXCLUSION (sites with controlContext ===
            // "unreachable" emit no facts — mirrors al-sem `extractCapabilities`).
            // R3a-2's projection never reads control_context, so this is additive.
            {
                let cc_params = crate::engine::l2::scope::extract_parameters(routine, source);
                let (_, attrs_json) =
                    crate::engine::l2::l2_workspace::collect_attributes(routine, source);
                let attr_names_lc: Vec<String> = attrs_json
                    .iter()
                    .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                    .map(|n| n.to_lowercase())
                    .collect();
                crate::engine::l2::control_context::apply_control_contexts(
                    &mut features,
                    &attr_names_lc,
                    &cc_params,
                );
            }

            // The routine's OWN record vars (params + locals), built first so they
            // take precedence over any same-named promoted global.
            let mut record_variables: Vec<L3RecordVariable> = features
                .record_variables
                .iter()
                .map(|rv| L3RecordVariable {
                    id: rv.id.clone(),
                    name: rv.name.clone(),
                    table_name: rv.table_name.clone(),
                    table_id: None,
                    is_parameter: rv.is_parameter,
                    parameter_index: rv.parameter_index,
                    temp_state: rv.temp_state.clone(),
                    scope: rv.scope.clone(),
                })
                .collect();
            // Task 3 (temp-state) PROMOTION + SHADOWING: append object-global record
            // vars, re-keyed to a per-routine id, but ONLY those whose (lowercased)
            // name is NOT already declared by the routine's own params/locals — the
            // routine's own var shadows the global (innermost wins). Skipping
            // shadowed globals keeps `record_variables` NAME-UNIQUE, which preserves
            // the documented pass-1 `var_index_by_name` last-wins invariant in
            // `record_types.rs` (a name-duplicated list would otherwise let the
            // global clobber the local in pass 1 — the WRONG result). Each promoted
            // global keeps `scope: Some("global")`, its `table_name`, and its
            // `temp_state` (the Known(true/false) the L2 walk could not derive).
            //
            // Perf: most objects have NO global record vars (the dominant case), so
            // skip the whole block — including the per-routine `own_names` build —
            // entirely when there is nothing to promote.
            if !object_global_record_vars.is_empty() {
                let own_names: std::collections::HashSet<String> = record_variables
                    .iter()
                    .map(|rv| rv.name.to_lowercase())
                    .collect();
                for g in &object_global_record_vars {
                    let lc = g.name.to_lowercase();
                    if own_names.contains(&lc) {
                        continue; // shadowed by the routine's own param/local
                    }
                    record_variables.push(L3RecordVariable {
                        id: format!("{}/rv/{}", routine_id, lc),
                        name: g.name.clone(),
                        table_name: g.table_name.clone(),
                        table_id: None,
                        is_parameter: g.is_parameter,
                        parameter_index: g.parameter_index,
                        temp_state: g.temp_state.clone(),
                        scope: g.scope.clone(),
                    });
                }
            }
            let record_operations = features
                .record_operations
                .iter()
                .map(|op| L3RecordOperation {
                    id: op.id.clone(),
                    op: op.op.clone(),
                    record_variable_name: op.record_variable_name.clone(),
                    record_variable_id: op.record_variable_id.clone(),
                    table_id: None,
                    temp_state: Some(op.temp_state.clone()),
                    field_arguments: op.field_arguments.clone(),
                    source_anchor: op.source_anchor.clone(),
                    loop_stack: op.loop_stack.clone(),
                    field_argument_infos: op.field_argument_infos.clone(),
                })
                .collect();
            let field_accesses = features.field_accesses.clone();
            let variables = features
                .variables
                .iter()
                .map(|v| L3Variable {
                    name: v.name.clone(),
                    declared_type: v.declared_type.clone(),
                    is_parameter: v.is_parameter,
                    parameter_index: v.parameter_index,
                    initializer: v.initializer.clone(),
                })
                .collect();

            // Re-extract the routine's own parameters + return type for the call
            // resolver (project_routine_features discards them after id hashing).
            // Reuse the SAME extractors the routine-id/signature-hash path uses so
            // arity/var-ness/type-text cannot drift.
            let parameters = crate::engine::l2::scope::extract_parameters(routine, source)
                .into_iter()
                .map(|p| L3Parameter {
                    index: p.index,
                    name: p.name,
                    type_text: p.type_text,
                    is_var: p.is_var,
                    is_record: p.is_record,
                    table_name: p.table_name,
                })
                .collect();
            let return_type = crate::engine::l2::get_return_type_text(routine, source);
            let mut call_sites = features.call_sites.clone();
            // RV-8 (Task 8): scope-honest `sourceKind`. The L2 binding builder
            // labels ANY non-parameter record-var arg `"local"` because scope is
            // not yet known at L2 (object globals are only PROMOTED into
            // `record_variables` here at L3). Now that promotion has run and the
            // record vars carry their `scope`, relabel a binding whose source
            // matches a PROMOTED GLOBAL (`scope == Some("global")`) from
            // `"local"` to `"global"`. Diagnostic-only: it removes the mislabel
            // without changing which args are persistable (a global is a real
            // caller var, persistable exactly like a local). Only "local"
            // bindings are eligible — "parameter" / "implicit-rec" / "expression"
            // are left untouched.
            {
                let global_rv_names_lc: std::collections::HashSet<&str> = record_variables
                    .iter()
                    .filter(|rv| rv.scope.as_deref() == Some("global"))
                    .map(|rv| rv.name.as_str())
                    .collect();
                if !global_rv_names_lc.is_empty() {
                    for cs in &mut call_sites {
                        for b in &mut cs.argument_bindings {
                            if b.source_kind != "local" {
                                continue;
                            }
                            if let Some(name_lc) = b.source_variable_name.as_deref() {
                                // `source_variable_name` is already lowercased at L2;
                                // promoted-global names are stored verbatim, so compare
                                // case-insensitively.
                                if global_rv_names_lc
                                    .iter()
                                    .any(|g| g.eq_ignore_ascii_case(name_lc))
                                {
                                    b.source_kind = "global".to_string();
                                }
                            }
                        }
                    }
                }
            }
            let operation_sites = features.operation_sites.clone();
            let statement_tree = features.statement_tree.clone();
            let loops = features.loops.clone();
            // d19 reads the L2 identifier-reference set (lowercased / sorted /
            // deduped exactly as L2 produced it); d20 reads the unreachable list.
            let identifier_references = features.identifier_references.clone();
            let unreachable_statements = features.unreachable_statements.clone();
            // d43 branch-slice surface: hasBranching + varAssignments + conditionReferences.
            let has_branching = features.has_branching;
            let var_assignments = features.var_assignments.clone();
            let condition_references = features.condition_references.clone();
            // The routine's OWN declaration anchor (al-sem routine-indexer.ts:419):
            // `syntax_kind` = node.type = "procedure" / "trigger_declaration".
            let source_anchor = anchor_from_node(routine, source_unit_id, cols);

            // L2 bodyAvailable / parseIncomplete — computed the SAME way the L2
            // projection does (routine-indexer.ts parity): a code_block body is
            // present; the routine subtree carries a tree-sitter ERROR node.
            let body_available = crate::engine::l2::find_code_block(routine).is_some();
            let parse_incomplete = routine.has_error();

            // StableRoutineId = `${stableObjectId}#${normalizedSignatureHash}`.
            // The hash reuses the same param/kind/return extraction as the internal
            // routine id (`routine_normalized_signature_hash`), so they cannot drift.
            let stable_object_id = to_stable_object_id(&object_id);
            let norm_hash = routine_normalized_signature_hash(routine, source).unwrap_or_default();
            let stable_routine_id = to_stable_routine_id_from_parts(&stable_object_id, &norm_hash);

            // Routine kind + structured attributes (the event-graph inputs). Reuse the
            // SAME L2 attribute indexing that produces the L2 projection's
            // `attributesParsed`, so the AttributeInfo arg shape (kind/value/qualifier/
            // member) cannot drift from R1.
            let kind = crate::engine::l2::l2_workspace::classify_kind(routine, source).to_string();
            let (_, attributes_parsed_json) =
                crate::engine::l2::l2_workspace::collect_attributes(routine, source);
            let attributes_parsed: Vec<super::al_attributes::AttributeInfo> =
                attributes_parsed_json
                    .into_iter()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
            // d32 scope gate: access modifier (`local`/`internal`/`protected`; None = public).
            let access_modifier =
                crate::engine::l2::l2_workspace::classify_access_modifier(routine, source);

            // E1: enclosing-member capture (additive — never serialized into a golden).
            // A member-trigger (parent is a member-bearing wrapper) gets the unescaped
            // logical member name + the WRAPPER node's source range (RE-2/RE-3/RE-4) and
            // `originatingObject` = the StableObjectId of the object decl in scope (the
            // EXTENSION object for an extension-declared trigger — RE-5). Procedures and
            // object-level triggers (OnRun / OnOpenPage) carry a non-member parent → all
            // `None`.
            let (enclosing_member, enclosing_member_range, originating_object) =
                match enclosing_member_of(member_parent, source) {
                    Some((member_name, wrapper)) => (
                        Some(member_name),
                        Some(anchor_from_node(wrapper, source_unit_id, cols)),
                        Some(stable_object_id.clone()),
                    ),
                    None => (None, None, None),
                };

            workspace.routines.push(L3Routine {
                id: routine_id,
                stable_routine_id,
                object_id: object_id.clone(),
                object_type: object_type.to_string(),
                name: rname,
                kind,
                attributes_parsed,
                app_guid: app_guid.to_string(),
                object_number,
                normalized_signature_hash: norm_hash,
                body_available,
                parse_incomplete,
                record_variables,
                record_operations,
                field_accesses,
                variables,
                parameters,
                access_modifier,
                return_type,
                call_sites,
                operation_sites,
                statement_tree,
                loops,
                source_anchor,
                identifier_references,
                unreachable_statements,
                has_branching,
                var_assignments,
                condition_references,
                enclosing_member,
                originating_object,
                enclosing_member_range,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Public assembly + resolution entry points.
// ---------------------------------------------------------------------------

/// Assemble the workspace L3 model from inline `(name, source)` files, in al-sem's
/// deterministic ingestion order (files sorted by name → per-file document order),
/// then run `resolve_record_types` + `merge_extension_fields`.
///
/// This is the offline entry point the vector test drives. Disk-backed workspaces
/// (the differential / dump in Task 3) sort discovered `.al` files by their
/// workspace-relative POSIX path — the same total order this reproduces.
pub fn assemble_and_resolve(
    files: &[(String, String)],
    app_guid: &str,
    model_instance_id: &str,
) -> L3Resolved {
    let mut workspace = assemble_workspace(files, app_guid, model_instance_id);
    resolve(&mut workspace);
    // Inline path: no disk `roots.config.json` ⇒ AST-only classifications, no infra diagnostics.
    // No disk `app.json` ⇒ primary_app = None.
    let (root_classifications, infra_diagnostics) =
        crate::engine::root_classification::compute_root_classifications(&workspace, None);
    L3Resolved {
        workspace,
        root_classifications,
        primary_app: None,
        infra_diagnostics,
    }
}

/// Assemble the workspace L3 model from inline `(name, source)` files WITHOUT
/// resolving — the parse/project half of [`assemble_and_resolve`]. The R2.5b
/// cross-app wiring appends dep entities to the result before calling `resolve`.
pub fn assemble_workspace(
    files: &[(String, String)],
    app_guid: &str,
    model_instance_id: &str,
) -> L3Workspace {
    // Deterministic ingestion order: sort files by name (the `ws:<name>` unit id
    // total order), then walk each file's objects in document order.
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut parser = Parser::new();
    parser
        .set_language(&crate::language::language())
        .expect("set tree-sitter language");

    let mut workspace = L3Workspace {
        objects: Vec::new(),
        tables: Vec::new(),
        routines: Vec::new(),
    };

    for (fname, source) in sorted {
        let Some(tree) = parser.parse(source, None) else {
            continue;
        };
        let source_unit_id = format!("ws:{fname}");
        let cols = Utf16Cols::new(source);
        project_file(
            tree.root_node(),
            source,
            app_guid,
            model_instance_id,
            &source_unit_id,
            &cols,
            &mut workspace,
        );
    }

    workspace
}

/// Assemble the workspace L3 model from inline `(source_unit_id, source)` units,
/// using the GIVEN `source_unit_id` verbatim for each file's anchors (instead of
/// the `ws:<name>` form `assemble_workspace` hardcodes). The R3a-4 dependency
/// producer needs this so each embedded `.al` file's op/callsite anchors carry the
/// al-sem `dep:<appGuid>:<relativePath>` source-unit id (the cited-evidence
/// `sourceFile` field), matching `ingestDependencyApp`'s embedded-source path.
///
/// Units are sorted by `source_unit_id` (the same total order
/// `iterateEmbeddedSource` yields: sorted-by-relative-path → here the unit ids are
/// `dep:<appGuid>:<sorted relativePath>`), then walked in document order. NOT
/// resolved — the caller runs `resolve`.
pub fn assemble_workspace_units(
    units: &[(String, String)],
    app_guid: &str,
    model_instance_id: &str,
) -> L3Workspace {
    let mut sorted: Vec<&(String, String)> = units.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut parser = Parser::new();
    parser
        .set_language(&crate::language::language())
        .expect("set tree-sitter language");

    let mut workspace = L3Workspace {
        objects: Vec::new(),
        tables: Vec::new(),
        routines: Vec::new(),
    };

    for (source_unit_id, source) in sorted {
        let Some(tree) = parser.parse(source, None) else {
            continue;
        };
        let cols = Utf16Cols::new(source);
        project_file(
            tree.root_node(),
            source,
            app_guid,
            model_instance_id,
            source_unit_id,
            &cols,
            &mut workspace,
        );
    }

    workspace
}

/// Convenience: assemble + resolve with the default model-instance id (`r0`).
pub fn assemble_and_resolve_default(files: &[(String, String)], app_guid: &str) -> L3Resolved {
    assemble_and_resolve(files, app_guid, MODEL_INSTANCE_ID_DEFAULT)
}

/// Disk-backed assemble + resolve over a workspace directory (the emitter +
/// differential entry point). Reuses L2's discovery so the file order, BOM
/// strip, app-guid read, and fail-closed layout detection match al-sem EXACTLY:
/// a sound workspace is ONE AL app (readable root `app.json` `id`, single
/// `app.json` excl. node_modules/.alpackages). The inline `ws:<relPosix>` unit
/// ids match `project_workspace`.
///
/// Returns `None` on an unsound / empty layout (fail-closed) — never throws.
pub fn assemble_and_resolve_workspace(
    workspace: &std::path::Path,
    model_instance_id: &str,
) -> Option<L3Resolved> {
    let resolved = {
        let mut ws = assemble_l3_workspace_from_disk(workspace, model_instance_id)?;
        resolve(&mut ws);
        // R4-F: classify AST roots, then overlay `<workspace>/roots.config.json`.
        // `workspace` is the root where the config lives (mirrors al-sem's
        // index.ts: `loadRootsConfig(workspaceRoot)`).
        let (root_classifications, infra_diagnostics) =
            crate::engine::root_classification::compute_root_classifications(&ws, Some(workspace));
        // Disk-backed path: read the primary app's identity from `app.json`.
        // Mirrors al-sem `model.identity.primaryApp`. Never throws — returns None
        // on unreadable / malformed app.json (fail-closed / engine-never-throws).
        let primary_app = read_primary_app_from_disk(workspace);
        L3Resolved {
            workspace: ws,
            root_classifications,
            primary_app,
            infra_diagnostics,
        }
    };
    // Empty fail-closed model (no objects/routines) → treat as not-analyzable.
    if resolved.workspace.objects.is_empty() && resolved.workspace.routines.is_empty() {
        return None;
    }
    Some(resolved)
}

/// Assemble the workspace L3 model from disk WITHOUT resolving — the pre-resolve
/// assembly half of [`assemble_and_resolve_workspace`], exposed so the R2.5b
/// cross-app wiring can append dep entities BEFORE running `resolve` over the
/// merged whole. Fail-closed: an unsound/empty native layout yields `None`
/// (readable root `app.json` `id`, single `app.json`, ≥1 readable `.al`).
pub fn assemble_l3_workspace_from_disk(
    workspace: &std::path::Path,
    model_instance_id: &str,
) -> Option<L3Workspace> {
    use crate::engine::l2::l2_workspace::{
        count_app_json, discover_al_files, read_al_source, read_root_app_guid,
    };

    // Fail-closed: need a readable root app.json with a string `id`, single app.
    let app_guid = read_root_app_guid(workspace)?;
    if count_app_json(workspace) > 1 {
        return None;
    }
    let discovered = discover_al_files(workspace).ok()?;

    // Build (relPosix, source) pairs in discovery (rel-posix-sorted) order; the
    // inline assembler re-sorts by name, which is the same total order.
    let mut files: Vec<(String, String)> = Vec::new();
    for f in &discovered {
        match read_al_source(&f.abs_path) {
            Ok(src) => files.push((f.rel_posix.clone(), src)),
            Err(e) => {
                eprintln!("warning: skipping {} (read error: {e})", f.rel_posix);
            }
        }
    }

    if files.is_empty() {
        return None;
    }

    Some(assemble_workspace(&files, &app_guid, model_instance_id))
}

/// Disk-backed convenience with the default model-instance id (`r0`).
pub fn assemble_and_resolve_workspace_default(workspace: &std::path::Path) -> Option<L3Resolved> {
    assemble_and_resolve_workspace(workspace, MODEL_INSTANCE_ID_DEFAULT)
}

/// Read the primary app's identity from the workspace root `app.json`.
/// Mirrors `run::read_workspace_apps` but returns `Option<App>` instead of `Vec<App>`.
/// Engine-never-throws: returns `None` on any read/parse failure.
fn read_primary_app_from_disk(
    workspace: &std::path::Path,
) -> Option<crate::engine::gate::app_attribution::App> {
    let text = std::fs::read_to_string(workspace.join("app.json")).ok()?;
    let v = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let app_guid = v
        .get("id")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())?;
    let publisher = v
        .get("publisher")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let version = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("0.0.0.0")
        .to_string();
    Some(crate::engine::gate::app_attribution::App {
        app_guid: app_guid.to_string(),
        publisher,
        name,
        version,
    })
}

/// Run the three L3 resolve sub-steps over an assembled workspace IN ORDER:
/// `build_symbol_table → resolve_record_types → merge_extension_fields`.
/// `tableId` is set by record-types and never re-touched by the merge.
///
/// L3-ONLY BOUNDARY (R2.5b Rev 2 #5): the input `L3Workspace`
/// (objects/tables/routines) is an L3-only merged index. Its entity structs carry
/// NO L4/cone/summary field — there is no `summary`, `intraAppCallEdges`,
/// `citedOperationEvidence`, `depOrderIndex`, capability-cone, or typed-edge field
/// anywhere on `L3Object`/`L3Table`/`L3Routine`. So L4 state CANNOT influence L3:
/// the boundary is enforced by the TYPE, not a runtime strip. When R2.5b feeds the
/// merged (workspace + `.app`-dep) index here (`deps::cross_app_l3`), the dep side
/// likewise comes from `project_abi_to_index`, which emits only these L3 structs.
/// DO NOT add an L4 field to these entity structs (it would breach the boundary the
/// `cross_app_l3_poison` test guards). NOTHING in `resolve` reads beyond them.
pub fn resolve(workspace: &mut L3Workspace) {
    let symbols = SymbolTable::build(&workspace.objects, &workspace.tables, &workspace.routines);

    // objectId → object, so a routine maps back to its owning object.
    use std::collections::HashMap;
    let object_by_id: HashMap<String, L3Object> = workspace
        .objects
        .iter()
        .map(|o| (o.id.clone(), o.clone()))
        .collect();

    for routine in &mut workspace.routines {
        let object = object_by_id.get(&routine.object_id);
        resolve_routine_record_types(routine, object, &symbols);
    }

    merge_extension_fields(workspace);
}

// ---------------------------------------------------------------------------
// Resolved-projection accessors (StableTableId form) — the test/dump surface.
// ---------------------------------------------------------------------------

/// Project an internal TableId (`${appGuid}/table/${n}`) to its StableTableId
/// (`${appGuid}:Table:${n}`). Mirrors al-sem `toStableTableId`.
pub fn to_stable_table_id(internal: &str) -> String {
    let parts: Vec<&str> = internal.split('/').collect();
    if parts.len() == 3 && parts[1] == "table" {
        format!("{}:Table:{}", parts[0], parts[2])
    } else {
        // Defensive: never panic in the engine. Return as-is (will fail compare).
        internal.to_string()
    }
}

/// A resolved workspace, exposing the StableTableId-projected lookups the parity
/// surface compares.
pub struct L3Resolved {
    pub workspace: L3Workspace,
    /// R4-F root classifications (`model.rootClassifications`): the AST root
    /// classifier overlaid with any `<workspace>/roots.config.json`. Computed at
    /// the disk-backed resolve entry (`assemble_and_resolve_workspace`, where the
    /// workspace root is known); the inline / cross-app constructors that have no
    /// disk config populate the AST-only set (empty config). Consumed by the L5
    /// `DetectorContext` (d50/d51) and the R4-F stable projection.
    pub root_classifications: Vec<crate::engine::root_classification::RootClassification>,
    /// The primary app's identity (`model.identity.primaryApp`): name / publisher /
    /// version read from the workspace `app.json`. Populated by the disk-backed
    /// assembly path (`assemble_and_resolve_workspace`); `None` in the inline /
    /// cross-app constructors (no disk `app.json` to read). Consumed by the html
    /// formatter's masthead/title (Stage A3) and any future envelope that needs the
    /// primary app description. Additive — `L3Resolved` is NOT serialized into any
    /// golden surface, so adding this field never moves a golden.
    pub primary_app: Option<crate::engine::gate::app_attribution::App>,
    /// Infrastructure diagnostics from the root-classification overlay (e.g.
    /// `kinds-mismatch` warnings from `roots.config.json`). Empty for inline /
    /// cross-app paths that have no disk config. Propagated to the JSON envelope.
    pub infra_diagnostics: Vec<crate::engine::root_classification::InfraDiagnostic>,
}

// ---------------------------------------------------------------------------
// Serde projection — the golden-shaped L3 record-type projection (matches
// `scripts/r2a-l3-projection.ts` / `*.l3rt.golden.json` EXACTLY).
//
// ALLOWLIST: only the record-type surface. Every field is built key-by-key, so
// later-gate fields (callGraph/eventGraph/coverage/typedEdges/resourceId/
// argumentBindings upgrades) cannot leak through. `tableId` is OMITTED when
// unresolved (serde `skip_serializing_if`), matching al-sem (never guessed).
// ---------------------------------------------------------------------------

/// A resolved record VARIABLE: lowercased name + resolved StableTableId (omitted
/// when unresolved).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PRecordVariable {
    pub name: String,
    #[serde(rename = "tableId", skip_serializing_if = "Option::is_none")]
    pub table_id: Option<String>,
}

/// A resolved record OPERATION keyed by operationId.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PRecordOperation {
    #[serde(rename = "operationId")]
    pub operation_id: String,
    pub op: String,
    #[serde(rename = "recordVariableName")]
    pub record_variable_name: String,
    #[serde(rename = "tableId", skip_serializing_if = "Option::is_none")]
    pub table_id: Option<String>,
}

/// Per-routine record-type resolution surface, keyed by StableRoutineId.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PRoutineRecordTypes {
    #[serde(rename = "stableRoutineId")]
    pub stable_routine_id: String,
    pub name: String,
    #[serde(rename = "objectType")]
    pub object_type: String,
    #[serde(rename = "recordVariables")]
    pub record_variables: Vec<PRecordVariable>,
    #[serde(rename = "recordOperations")]
    pub record_operations: Vec<PRecordOperation>,
}

/// One MERGED field (post `merge_extension_fields`) in stable id form.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PMergedField {
    #[serde(rename = "fieldNumber")]
    pub field_number: i64,
    pub name: String,
    #[serde(rename = "dataType")]
    pub data_type: String,
    #[serde(rename = "fieldClass")]
    pub field_class: String,
    #[serde(rename = "declaringObjectId")]
    pub declaring_object_id: String,
}

/// A Table with its MERGED field set (base + TableExtension fields).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PTableRecordTypes {
    #[serde(rename = "stableTableId")]
    pub stable_table_id: String,
    pub name: String,
    pub fields: Vec<PMergedField>,
}

/// The full L3 record-type projection — the golden document shape.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct L3RecordTypeProjection {
    pub tables: Vec<PTableRecordTypes>,
    pub routines: Vec<PRoutineRecordTypes>,
}

impl L3Resolved {
    /// Project the resolved workspace to the golden-shaped L3 record-type
    /// projection. Tables sorted by StableTableId; routines by StableRoutineId;
    /// record vars by (name, tableId); record ops by operationId; fields by
    /// (fieldNumber, name) — matching `scripts/r2a-l3-projection.ts`.
    pub fn project(&self) -> L3RecordTypeProjection {
        let mut tables: Vec<PTableRecordTypes> = self
            .workspace
            .tables
            .iter()
            .map(|t| {
                let mut fields: Vec<PMergedField> = t
                    .fields
                    .iter()
                    .map(|f| PMergedField {
                        field_number: f.field_number,
                        name: f.name.clone(),
                        data_type: f.data_type.clone(),
                        field_class: f.field_class.clone(),
                        declaring_object_id: to_stable_object_id(&f.declaring_object_id),
                    })
                    .collect();
                fields.sort_by(|a, b| {
                    a.field_number
                        .cmp(&b.field_number)
                        .then_with(|| a.name.cmp(&b.name))
                });
                PTableRecordTypes {
                    stable_table_id: to_stable_table_id(&t.id),
                    name: t.name.clone(),
                    fields,
                }
            })
            .collect();
        tables.sort_by(|a, b| a.stable_table_id.cmp(&b.stable_table_id));

        let mut routines: Vec<PRoutineRecordTypes> = self
            .workspace
            .routines
            .iter()
            .map(|r| {
                let mut record_variables: Vec<PRecordVariable> = r
                    .record_variables
                    .iter()
                    .map(|v| PRecordVariable {
                        name: v.name.to_lowercase(),
                        table_id: v.table_id.as_deref().map(to_stable_table_id),
                    })
                    .collect();
                record_variables.sort_by(|a, b| {
                    a.name.cmp(&b.name).then_with(|| {
                        a.table_id
                            .clone()
                            .unwrap_or_default()
                            .cmp(&b.table_id.clone().unwrap_or_default())
                    })
                });

                let mut record_operations: Vec<PRecordOperation> = r
                    .record_operations
                    .iter()
                    .map(|op| PRecordOperation {
                        operation_id: op.id.clone(),
                        op: op.op.clone(),
                        record_variable_name: op.record_variable_name.to_lowercase(),
                        table_id: op.table_id.as_deref().map(to_stable_table_id),
                    })
                    .collect();
                record_operations.sort_by(|a, b| a.operation_id.cmp(&b.operation_id));

                PRoutineRecordTypes {
                    stable_routine_id: r.stable_routine_id.clone(),
                    name: r.name.clone(),
                    object_type: r.object_type.clone(),
                    record_variables,
                    record_operations,
                }
            })
            .collect();
        routines.sort_by(|a, b| a.stable_routine_id.cmp(&b.stable_routine_id));

        L3RecordTypeProjection { tables, routines }
    }
}

/// A merged field projected for comparison (StableObjectId declaring provenance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedField {
    pub field_number: i64,
    pub name: String,
    pub data_type: String,
    pub field_class: String,
    pub declaring_object_id: String,
}

/// A routine view exposing resolved record var / op StableTableIds by name.
pub struct RoutineView<'a> {
    routine: &'a L3Routine,
}

impl L3Resolved {
    /// Find a routine by name (first match in assembled order).
    pub fn routine_by_name(&self, name: &str) -> Option<RoutineView<'_>> {
        self.workspace
            .routines
            .iter()
            .find(|r| r.name == name)
            .map(|routine| RoutineView { routine })
    }

    /// Find a table by name (case-insensitive, LAST-wins — matching the symbol
    /// table the resolution queried).
    pub fn table_by_name(&self, name: &str) -> Option<TableView<'_>> {
        let want = name.to_lowercase();
        let mut found = None;
        for t in &self.workspace.tables {
            if t.name.to_lowercase() == want {
                found = Some(t); // LAST-wins
            }
        }
        found.map(|table| TableView { table })
    }
}

impl RoutineView<'_> {
    /// Resolved StableTableId for the named record variable, or None if unresolved
    /// / absent.
    pub fn record_var_table_id(&self, name: &str) -> Option<String> {
        let want = name.to_lowercase();
        self.routine
            .record_variables
            .iter()
            .find(|v| v.name.to_lowercase() == want)
            .and_then(|v| v.table_id.as_deref().map(to_stable_table_id))
    }

    pub fn record_var_count(&self) -> usize {
        self.routine.record_variables.len()
    }

    /// The `scope` (`"local"` | `"parameter"` | `"global"`) of the named record
    /// variable, or None if absent / unset. Test-facing accessor for the Task 3
    /// global-promotion path.
    pub fn record_var_scope(&self, name: &str) -> Option<String> {
        let want = name.to_lowercase();
        self.routine
            .record_variables
            .iter()
            .find(|v| v.name.to_lowercase() == want)
            .and_then(|v| v.scope.clone())
    }

    /// The resolved `temp_state` Known value of the named record variable, or None
    /// if the var is absent or its temp_state is not `known`.
    pub fn record_var_temp_known(&self, name: &str) -> Option<bool> {
        let want = name.to_lowercase();
        self.routine
            .record_variables
            .iter()
            .find(|v| v.name.to_lowercase() == want)
            .and_then(|v| v.temp_state_known_value())
    }

    /// The resolved `temp_state` Known value of the FIRST record OP on the named
    /// record variable, or None if absent / not `known`. Test-facing accessor for
    /// the Task 3 member-var op temp_state backfill. Returns the FIRST matching op's
    /// state (walk order) — sufficient for the single-op-per-var test fixtures.
    pub fn first_record_op_temp_known(&self, var_name: &str) -> Option<bool> {
        let want = var_name.to_lowercase();
        self.routine
            .record_operations
            .iter()
            .find(|op| op.record_variable_name.to_lowercase() == want)
            .and_then(|op| op.temp_state.as_ref())
            .and_then(|ts| if ts.kind == "known" { ts.value } else { None })
    }

    /// All record ops in walk order as `(op, recordVariableName, Option<StableTableId>)`.
    pub fn record_ops(&self) -> Vec<(String, String, Option<String>)> {
        self.routine
            .record_operations
            .iter()
            .map(|op| {
                (
                    op.op.clone(),
                    op.record_variable_name.clone(),
                    op.table_id.as_deref().map(to_stable_table_id),
                )
            })
            .collect()
    }
}

/// A table view exposing the merged fields (StableObjectId provenance).
pub struct TableView<'a> {
    table: &'a L3Table,
}

impl TableView<'_> {
    pub fn stable_table_id(&self) -> String {
        to_stable_table_id(&self.table.id)
    }

    /// True when the table is declared `TableType = Temporary` (Task 4 Part A).
    pub fn is_temporary(&self) -> bool {
        self.table.is_temporary
    }

    /// Merged fields in stored order, declaringObjectId projected to StableObjectId.
    pub fn merged_fields(&self) -> Vec<ProjectedField> {
        self.table
            .fields
            .iter()
            .map(|f| ProjectedField {
                field_number: f.field_number,
                name: f.name.clone(),
                data_type: f.data_type.clone(),
                field_class: f.field_class.clone(),
                declaring_object_id: to_stable_object_id(&f.declaring_object_id),
            })
            .collect()
    }
}
