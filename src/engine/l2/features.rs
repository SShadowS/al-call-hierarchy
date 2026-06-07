//! R1a L2 feature projection — serde types matching al-sem's ALLOWLISTED L2
//! shape EXACTLY (`scripts/r1a-l2-projection.ts`).
//!
//! These types are the parity surface against the committed
//! `<fixture>.l2.golden.json` files and the `l2-vectors.json` family vectors.
//!
//! R1c: `order` (OperationOrder) on each op/callsite + `scopeFrames` on the
//! routine features ARE now declared + emitted (no longer forbidden).
//!
//! FORBIDDEN fields are STRUCTURALLY ABSENT — they are not declared here, so a
//! stray field can never serialize:
//!   - CapabilityFact (R1d)
//!   - tableId on RecordVariable/RecordOperation/VariableSymbol (L3 Phase-2)
//!   - resourceId (L3 Phase-2)
//!   - resolver-upgraded argumentBindings fields: calleeParameterIsVar,
//!     bindingResolution, sourceTableId (L3 call-resolver).
//!
//! DROPPED (mirrors the TS projection): CFN-node sourceAnchor (the skeleton is
//! shape + id refs only), and enclosingRoutineId on every anchor (it embeds the
//! modelInstanceId-dependent internal RoutineId).
//!
//! `skip_serializing_if = "Option::is_none"` mirrors the TS pattern of only
//! emitting a key when the value is defined — so the JSON shape matches the
//! golden's "optional keys absent" convention exactly.

use super::operation_order::{OperationOrder, ScopeFrame};
use serde::{Deserialize, Serialize};

/// A projected source anchor. Columns are tree-sitter byte columns (which match
/// al-sem's web-tree-sitter columns byte-for-byte — see `node_util::Utf16Cols`);
/// `enclosingRoutineId` is dropped (it embeds the modelInstanceId-dependent id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PAnchor {
    #[serde(rename = "sourceUnitId")]
    pub source_unit_id: String,
    #[serde(rename = "startLine")]
    pub start_line: u32,
    #[serde(rename = "startColumn")]
    pub start_column: u32,
    #[serde(rename = "endLine")]
    pub end_line: u32,
    #[serde(rename = "endColumn")]
    pub end_column: u32,
    #[serde(rename = "syntaxKind")]
    pub syntax_kind: String,
}

/// Structured expression classification (`model/expression.ts`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PExpressionInfo {
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
}

/// Temp-state of a record variable / op.
///
/// `{ kind: "known", value }` | `{ kind: "parameter-dependent", parameterIndex }`
/// | `{ kind: "unknown" }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PTempState {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<bool>,
    #[serde(rename = "parameterIndex", skip_serializing_if = "Option::is_none")]
    pub parameter_index: Option<u32>,
}

/// Structured Callee classification (matches `model/callee.ts`).
///
/// Untagged so the bare / member / object-run / unknown shapes serialize as
/// flat objects (the TS `Callee` union is structural, no discriminant key beyond
/// `kind`). Each variant carries its own field set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum PCallee {
    #[serde(rename = "bare")]
    Bare { name: String },
    #[serde(rename = "member")]
    Member { receiver: String, method: String },
    #[serde(rename = "object-run")]
    ObjectRun {
        #[serde(rename = "objectKind")]
        object_kind: String,
        #[serde(rename = "targetType")]
        target_type: String,
        #[serde(rename = "targetRef", skip_serializing_if = "Option::is_none")]
        target_ref: Option<String>,
        #[serde(rename = "targetIsName")]
        target_is_name: bool,
    },
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PCallArgumentBinding {
    #[serde(rename = "parameterIndex")]
    pub parameter_index: u32,
    #[serde(rename = "sourceKind")]
    pub source_kind: String,
    #[serde(rename = "sourceVariableName", skip_serializing_if = "Option::is_none")]
    pub source_variable_name: Option<String>,
    #[serde(
        rename = "sourceRecordVariableId",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_record_variable_id: Option<String>,
    #[serde(
        rename = "sourceParameterIndex",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_parameter_index: Option<u32>,
    #[serde(
        rename = "callerSourceParameterIsVar",
        skip_serializing_if = "Option::is_none"
    )]
    pub caller_source_parameter_is_var: Option<bool>,
    #[serde(rename = "sourceTempState", skip_serializing_if = "Option::is_none")]
    pub source_temp_state: Option<PTempState>,
    #[serde(rename = "argumentAnchor")]
    pub argument_anchor: PAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PCallSite {
    pub id: String,
    #[serde(rename = "operationId")]
    pub operation_id: String,
    #[serde(rename = "calleeText")]
    pub callee_text: String,
    pub callee: PCallee,
    #[serde(rename = "argumentTexts")]
    pub argument_texts: Vec<String>,
    #[serde(rename = "argumentInfos")]
    pub argument_infos: Vec<PExpressionInfo>,
    #[serde(rename = "argumentBindings")]
    pub argument_bindings: Vec<PCallArgumentBinding>,
    #[serde(rename = "loopStack")]
    pub loop_stack: Vec<String>,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: PAnchor,
    #[serde(rename = "resultConsumed", skip_serializing_if = "Option::is_none")]
    pub result_consumed: Option<bool>,
    #[serde(
        rename = "objectRunReturnUsed",
        skip_serializing_if = "Option::is_none"
    )]
    pub object_run_return_used: Option<bool>,
    #[serde(rename = "underAsserterror", skip_serializing_if = "Option::is_none")]
    pub under_asserterror: Option<bool>,
    /// R1b control-context lattice value (kebab-case string). ABSENT when the
    /// site has no entry (TryFunction / no body / unknown) — matching al-sem's
    /// "assign only when defined" convention.
    #[serde(rename = "controlContext", skip_serializing_if = "Option::is_none")]
    pub control_context: Option<String>,
    /// R1c operation-order index entry. ABSENT when the walk produced no entry
    /// (symbol-only / no-body / TryFunction) — matching al-sem's
    /// "assign only when defined" convention.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<OperationOrder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct POperationSite {
    pub id: String,
    pub kind: String,
    #[serde(rename = "loopStack")]
    pub loop_stack: Vec<String>,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: PAnchor,
    #[serde(rename = "underAsserterror", skip_serializing_if = "Option::is_none")]
    pub under_asserterror: Option<bool>,
    /// R1b control-context lattice value (kebab-case string). ABSENT when the
    /// site has no entry (TryFunction / no body / unknown).
    #[serde(rename = "controlContext", skip_serializing_if = "Option::is_none")]
    pub control_context: Option<String>,
    /// R1c operation-order index entry. ABSENT when the walk produced no entry.
    /// For `error-call` ops, populated by the emitter's source-range post-pass
    /// (it inherits the paired callsite's order verbatim).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<OperationOrder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PRecordOperation {
    pub id: String,
    pub op: String,
    #[serde(rename = "recordVariableName")]
    pub record_variable_name: String,
    #[serde(rename = "recordVariableId", skip_serializing_if = "Option::is_none")]
    pub record_variable_id: Option<String>,
    #[serde(rename = "tempState")]
    pub temp_state: PTempState,
    #[serde(rename = "fieldArguments", skip_serializing_if = "Option::is_none")]
    pub field_arguments: Option<Vec<String>>,
    #[serde(rename = "fieldArgumentInfos", skip_serializing_if = "Option::is_none")]
    pub field_argument_infos: Option<Vec<PExpressionInfo>>,
    #[serde(rename = "loopStack")]
    pub loop_stack: Vec<String>,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: PAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PLoop {
    pub id: String,
    #[serde(rename = "type")]
    pub loop_type: String,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: PAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PRecordVariable {
    pub id: String,
    pub name: String,
    #[serde(rename = "tableName", skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
    #[serde(rename = "tempState")]
    pub temp_state: PTempState,
    #[serde(rename = "isParameter")]
    pub is_parameter: bool,
    #[serde(rename = "parameterIndex", skip_serializing_if = "Option::is_none")]
    pub parameter_index: Option<u32>,
}

/// Variable initializer — a one-hop `ValueSource` projection. Kept as raw JSON
/// because R1a does not constrain its internal shape beyond "matches the TS
/// `VariableSymbol.initializer` value verbatim".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PVariableSymbol {
    pub name: String,
    #[serde(rename = "declaredType")]
    pub declared_type: String,
    pub scope: String,
    #[serde(rename = "isParameter")]
    pub is_parameter: bool,
    #[serde(rename = "parameterIndex", skip_serializing_if = "Option::is_none")]
    pub parameter_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initializer: Option<serde_json::Value>,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: PAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PFieldAccess {
    #[serde(rename = "recordVariableName")]
    pub record_variable_name: String,
    #[serde(rename = "fieldName")]
    pub field_name: String,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: PAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PVarAssignment {
    #[serde(rename = "lhsName")]
    pub lhs_name: String,
    #[serde(rename = "rhsLiteralValue", skip_serializing_if = "Option::is_none")]
    pub rhs_literal_value: Option<String>,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: PAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PConditionReference {
    pub identifier: String,
    #[serde(rename = "conditionKind")]
    pub condition_kind: String,
    #[serde(rename = "statementAnchor")]
    pub statement_anchor: PAnchor,
    #[serde(rename = "referenceAnchor")]
    pub reference_anchor: PAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PUnreachableStatement {
    pub id: String,
    #[serde(rename = "exitKind")]
    pub exit_kind: String,
    #[serde(rename = "exitAnchor")]
    pub exit_anchor: PAnchor,
    #[serde(rename = "unreachableAnchor")]
    pub unreachable_anchor: PAnchor,
}

/// A simple boolean-guard condition recognized on `if` nodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PConditionGuard {
    pub identifier: String,
    pub polarity: String,
}

/// Normalized CFN skeleton node — kind, child/else structure, op/callsite refs,
/// conditionGuard, ordered conditionLeaves. sourceAnchor DROPPED.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PCFNNode {
    pub kind: String,
    #[serde(rename = "operationId", skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(rename = "callsiteId", skip_serializing_if = "Option::is_none")]
    pub callsite_id: Option<String>,
    #[serde(rename = "conditionGuard", skip_serializing_if = "Option::is_none")]
    pub condition_guard: Option<PConditionGuard>,
    #[serde(rename = "conditionLeaves", skip_serializing_if = "Option::is_none")]
    pub condition_leaves: Option<Vec<PCFNNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<PCFNNode>>,
    #[serde(rename = "elseChildren", skip_serializing_if = "Option::is_none")]
    pub else_children: Option<Vec<PCFNNode>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PFeatures {
    pub loops: Vec<PLoop>,
    #[serde(rename = "operationSites")]
    pub operation_sites: Vec<POperationSite>,
    #[serde(rename = "recordOperations")]
    pub record_operations: Vec<PRecordOperation>,
    #[serde(rename = "callSites")]
    pub call_sites: Vec<PCallSite>,
    #[serde(rename = "fieldAccesses")]
    pub field_accesses: Vec<PFieldAccess>,
    #[serde(rename = "recordVariables")]
    pub record_variables: Vec<PRecordVariable>,
    #[serde(rename = "nestingDepth")]
    pub nesting_depth: u32,
    #[serde(rename = "hasBranching")]
    pub has_branching: bool,
    #[serde(rename = "unreachableStatements")]
    pub unreachable_statements: Vec<PUnreachableStatement>,
    #[serde(rename = "identifierReferences")]
    pub identifier_references: Vec<String>,
    pub variables: Vec<PVariableSymbol>,
    #[serde(rename = "varAssignments")]
    pub var_assignments: Vec<PVarAssignment>,
    #[serde(rename = "conditionReferences")]
    pub condition_references: Vec<PConditionReference>,
    #[serde(rename = "statementTree", skip_serializing_if = "Option::is_none")]
    pub statement_tree: Option<PCFNNode>,
    /// R1c scope-frame table. OMITTED when empty (TryFunction / no body), but
    /// PRESENT (carrying the root "block" frame) when a body tree exists even with
    /// zero orders — mirrors al-sem `routine-indexer.ts:398`
    /// (`...(scopeFrames.length > 0 ? { scopeFrames } : {})`).
    #[serde(rename = "scopeFrames", default, skip_serializing_if = "Vec::is_empty")]
    pub scope_frames: Vec<ScopeFrame>,
}

/// A projected routine envelope (for the golden files; metadata prerequisite).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PRoutine {
    #[serde(rename = "stableRoutineId")]
    pub stable_routine_id: String,
    pub name: String,
    pub kind: String,
    pub attributes: Vec<String>,
    #[serde(rename = "attributesParsed")]
    pub attributes_parsed: Vec<serde_json::Value>,
    #[serde(rename = "accessModifier", skip_serializing_if = "Option::is_none")]
    pub access_modifier: Option<String>,
    #[serde(rename = "bodyAvailable")]
    pub body_available: bool,
    #[serde(rename = "parseIncomplete")]
    pub parse_incomplete: bool,
    pub features: PFeatures,
}

/// A projected object envelope (for the golden files; metadata prerequisite).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PObject {
    #[serde(rename = "stableObjectId")]
    pub stable_object_id: String,
    pub name: String,
    #[serde(rename = "objectType")]
    pub object_type: String,
    #[serde(rename = "objectSubtype", skip_serializing_if = "Option::is_none")]
    pub object_subtype: Option<String>,
    #[serde(rename = "pageType", skip_serializing_if = "Option::is_none")]
    pub page_type: Option<String>,
    #[serde(rename = "sourceTableName", skip_serializing_if = "Option::is_none")]
    pub source_table_name: Option<String>,
    #[serde(
        rename = "inherentCommitBehavior",
        skip_serializing_if = "Option::is_none"
    )]
    pub inherent_commit_behavior: Option<String>,
}

/// Full L2 projection of a workspace (pre-resolve). Top-level routines sorted by
/// StableRoutineId, objects by StableObjectId.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct L2Projection {
    pub objects: Vec<PObject>,
    pub routines: Vec<PRoutine>,
}
