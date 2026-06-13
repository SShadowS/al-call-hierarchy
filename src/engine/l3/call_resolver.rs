//! L3 call resolver (R2b Task 3) — faithful port of al-sem's `resolveCalls` /
//! `resolveCallSite` / `resolveByNameAndArity` / `resolveInterfaceDispatch` /
//! `upgradeBindings` from `src/resolve/call-resolver.ts`.
//!
//! Resolves every call site in the assembled L3 workspace into one or more
//! `CallEdge`s (interface dispatch is MULTI-edge), mutating each callsite's
//! argument bindings exactly ONCE (`upgrade_bindings`) once the callee is known.
//! Unresolved calls are DATA (an edge with no `to` and a non-"resolved"
//! resolution), never a silent gap.
//!
//! All ids on the produced edges are INTERNAL ids (routine id / callsite id /
//! operation id). The dump / vector test projects them to StableRoutineId and
//! groups multi-edge callsites + lifts `dispatchMeta` to the group level.

use super::al_builtins::global_builtin_disposition;
use super::implicit_edges::build_implicit_trigger_edges;
use super::l3_workspace::{L3Routine, L3Workspace};
use super::receiver::simple_receiver_name;
use super::static_arg::static_arg_type;
use super::symbol_table::SymbolTable;
use super::type_ref::{parse_object_type_ref, ObjectKind};
use super::type_rel::{type_relation, TypeRelation};
use crate::engine::l2::features::PCallSite;
use crate::engine::l3::taxonomy::{DispatchKind, Resolution};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Edge model (the resolver's internal-id shape).
// ---------------------------------------------------------------------------

/// Interface-dispatch metadata, attached to ONLY the first emitted edge (after
/// the sort-by-`to`) or the single unknown edge. The dump lifts this to the
/// callsite-group level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchMeta {
    pub interface_name: String,
    pub total_impls: usize,
    /// (internal objectId, reason) for each impl that did not resolve.
    pub unresolved_impls: Vec<(String, String)>,
    /// internal object ids of enum implementers (metadata only).
    pub enum_implementers: Vec<String>,
}

/// An external (out-of-index) type reference on a member edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTypeRef {
    pub kind: String,
    pub name: String,
}

/// Why a `resolution == "unknown"` edge could not be resolved. DIAGNOSTIC-only
/// metadata (never projected to a golden — `CallEdge` is not `Serialize`); it lets
/// `aldump --l3-unknown-breakdown` attribute the residual real-`unknown` rate to
/// its causes, which is the work-list for the later typed-resolution phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// Bare call, not own-object, not a global builtin.
    BareUnresolved,
    /// Member call whose receiver is a compound expression (`a.b.M()`, `(x).M()`,
    /// indexed) — `simple_receiver_name` declined it.
    CompoundReceiver,
    /// Member call whose receiver name is not a local/param/global in the routine
    /// (object globals not captured, `CurrPage`/`CurrReport`, return-value chains).
    UntrackedReceiver,
    /// Member call on a `Record`-typed receiver whose method is NOT a builtin — a
    /// real table procedure (resolvable by the later Record-dispatch phase).
    RecordTableProcedure,
    /// Member call on a RecordRef/FieldRef/KeyRef/framework receiver whose method is
    /// not in the intrinsic catalog (a catalog gap to fill).
    FrameworkMethodNotInCatalog,
    /// Member call whose declared receiver type is a primitive / Variant /
    /// unrecognized type (no object, no catalog kind).
    NonObjectReceiverType,
    /// Member call on an enum-typed receiver (enum statics are not callable here).
    EnumStatic,
    /// The L2 callee itself could not be parsed (`PCallee::Unknown`).
    CalleeUnknown,
    /// Interface dispatch where NO implementer resolved (open-world / no impls).
    InterfaceNoImpl,
    /// Object-run whose target is a dynamic variable (not a static ref) -- the
    /// dispatch kind is `dynamic`; target is unknowable without runtime info.
    DynamicObjectRunTarget,
}

impl UnknownReason {
    /// Stable kebab-case label for the diagnostic breakdown histogram.
    pub fn label(self) -> &'static str {
        match self {
            UnknownReason::BareUnresolved => "bare-unresolved",
            UnknownReason::CompoundReceiver => "compound-receiver",
            UnknownReason::UntrackedReceiver => "untracked-receiver",
            UnknownReason::RecordTableProcedure => "record-table-procedure",
            UnknownReason::FrameworkMethodNotInCatalog => "framework-method-not-in-catalog",
            UnknownReason::NonObjectReceiverType => "non-object-receiver-type",
            UnknownReason::EnumStatic => "enum-static",
            UnknownReason::CalleeUnknown => "callee-unknown",
            UnknownReason::InterfaceNoImpl => "interface-no-impl",
            UnknownReason::DynamicObjectRunTarget => "dynamic-objectrun-target",
        }
    }
}

/// A resolved (or unresolved) call edge. Ids are INTERNAL until projected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEdge {
    pub from: String,
    pub to: Option<String>,
    pub callsite_id: String,
    pub operation_id: String,
    pub dispatch_kind: DispatchKind,
    pub resolution: Resolution,
    /// candidates (internal routine ids), for ambiguous / member-not-found.
    pub candidates: Option<Vec<String>>,
    pub external_type_ref: Option<ExternalTypeRef>,
    /// method-dispatch receiver's declared type.
    pub receiver_type: Option<String>,
    pub dispatch_meta: Option<DispatchMeta>,
    /// For `FrameworkMethodNotInCatalog` unknown edges: the `"Kind::method_lc"`
    /// detail string that identifies the catalog gap. `None` on all other edges.
    pub unknown_method_name: Option<String>,
}

impl CallEdge {
    pub(crate) fn base(from: &str, callsite_id: &str, operation_id: &str) -> CallEdge {
        CallEdge {
            from: from.to_string(),
            to: None,
            callsite_id: callsite_id.to_string(),
            operation_id: operation_id.to_string(),
            dispatch_kind: DispatchKind::Unresolved,
            resolution: Resolution::Unknown(UnknownReason::CalleeUnknown),
            candidates: None,
            external_type_ref: None,
            receiver_type: None,
            dispatch_meta: None,
            unknown_method_name: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Upgraded-binding side table (the `upgradeBindings` mutation, captured out of
// band because the L3 PCallArgumentBinding does not carry the upgrade fields).
// ---------------------------------------------------------------------------

/// The post-upgrade state of one argument binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradedBinding {
    pub parameter_index: u32,
    pub callee_parameter_is_var: bool,
    /// "non-record-arg" | "unresolved-callee" | "resolved" | "ambiguous".
    pub binding_resolution: String,
}

/// Per-callsite upgraded bindings. `upgraded` guards `upgrade_bindings` so it
/// runs EXACTLY once per callsite (reproducing al-sem's double-upgrade guard).
struct BindingState {
    bindings: Vec<UpgradedBinding>,
}

/// A diagnostic (the resolver only emits the double-upgrade warning).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: String,
    pub stage: String,
    pub message: String,
}

/// Derive the INITIAL bindingResolution for an L3 callsite's bindings, matching
/// al-sem's `intraprocedural-body.ts` construction:
///   - non-identifier arg (sourceKind "expression") → "non-record-arg"
///   - identifier bound to a record variable → "unresolved-callee" (upgradable)
///   - any other identifier (param / implicit-rec / unknown) → "non-record-arg"
///
/// `calleeParameterIsVar` starts `false` (upgraded later).
fn initial_binding_state(call_site: &PCallSite) -> BindingState {
    let bindings = call_site
        .argument_bindings
        .iter()
        .map(|b| {
            let resolution = if b.source_kind == "expression" {
                "non-record-arg"
            } else if b.source_record_variable_id.is_some() {
                "unresolved-callee"
            } else {
                "non-record-arg"
            };
            UpgradedBinding {
                parameter_index: b.parameter_index,
                callee_parameter_is_var: false,
                binding_resolution: resolution.to_string(),
            }
        })
        .collect();
    BindingState { bindings }
}

/// Upgrade a callsite's bindings with callee-side var-ness once the callee is
/// known. Sets `bindingResolution = "resolved"` + `calleeParameterIsVar` for any
/// binding not already "non-record-arg". Returns a diagnostic on double-upgrade
/// (and skips), reproducing al-sem's non-idempotence guard.
fn upgrade_bindings(
    state: &mut BindingState,
    callee: &L3Routine,
    callsite_id: &str,
) -> Option<Diagnostic> {
    for b in &state.bindings {
        if b.binding_resolution == "resolved" || b.binding_resolution == "ambiguous" {
            return Some(Diagnostic {
                severity: "warning".to_string(),
                stage: "resolve".to_string(),
                message: format!(
                    "call-resolver: argumentBindings for callsite {callsite_id} already upgraded (double-upgrade); skipping re-entrant resolution"
                ),
            });
        }
    }
    for (i, b) in state.bindings.iter_mut().enumerate() {
        if b.binding_resolution == "non-record-arg" {
            continue;
        }
        let Some(param) = callee.parameters.get(i) else {
            continue; // arity mismatch — leave defaults
        };
        b.callee_parameter_is_var = param.is_var;
        b.binding_resolution = "resolved".to_string();
    }
    None
}

/// Mark all record-arg bindings "ambiguous" (leave "non-record-arg" untouched).
fn mark_bindings_ambiguous(state: &mut BindingState) {
    for b in &mut state.bindings {
        if b.binding_resolution == "non-record-arg" {
            continue;
        }
        b.binding_resolution = "ambiguous".to_string();
    }
}

// ---------------------------------------------------------------------------
// Arity-aware overload resolution.
// ---------------------------------------------------------------------------

enum ArityResolution<'a> {
    Resolved(&'a L3Routine),
    NotFound,
    NoArityMatch(Vec<&'a L3Routine>),
    Ambiguous(Vec<&'a L3Routine>),
}

/// Argument-type-aware overload tiebreak: drop ONLY candidates an inferred arg
/// type proves incompatible; resolve iff exactly one survives. Faithful port of
/// `disambiguateByArgTypes`.
fn disambiguate_by_arg_types<'a>(
    candidates: &[&'a L3Routine],
    caller: &L3Routine,
    call_site: &PCallSite,
    symbols: &SymbolTable,
) -> Option<&'a L3Routine> {
    let arg_types: Vec<Option<String>> = (0..call_site.argument_bindings.len())
        .map(|i| static_arg_type(caller, call_site, i, symbols))
        .collect();
    let survivors: Vec<&L3Routine> = candidates
        .iter()
        .copied()
        .filter(|cand| {
            arg_types.iter().enumerate().all(|(i, arg_type)| {
                let Some(arg_type) = arg_type else {
                    return true; // unknown position eliminates nothing
                };
                let Some(param) = cand.parameters.get(i) else {
                    return true;
                };
                type_relation(arg_type, &param.type_text) != TypeRelation::DefinitelyIncompatible
            })
        })
        .collect();
    if survivors.len() == 1 {
        Some(survivors[0])
    } else {
        None
    }
}

/// Resolve a call to `method_name` in `object_id` by name + exact arity, with
/// arg-type disambiguation when >1 same-arity candidates. Faithful port of
/// `resolveByNameAndArity`.
fn resolve_by_name_and_arity<'a>(
    symbols: &'a SymbolTable,
    object_id: &str,
    method_name: &str,
    caller: &L3Routine,
    call_site: &PCallSite,
) -> ArityResolution<'a> {
    let arg_count = call_site.argument_bindings.len();
    let matches = symbols.routines_in_object_by_name(object_id, method_name);
    if matches.is_empty() {
        return ArityResolution::NotFound;
    }
    let arity_matches: Vec<&L3Routine> = matches
        .iter()
        .copied()
        .filter(|m| m.parameters.len() == arg_count)
        .collect();
    if arity_matches.len() == 1 {
        return ArityResolution::Resolved(arity_matches[0]);
    }
    if arity_matches.is_empty() {
        return ArityResolution::NoArityMatch(matches);
    }
    if let Some(narrowed) = disambiguate_by_arg_types(&arity_matches, caller, call_site, symbols) {
        return ArityResolution::Resolved(narrowed);
    }
    ArityResolution::Ambiguous(arity_matches)
}

/// Map an object-run objectKind to its dispatch kind.
fn object_run_dispatch_kind(object_kind: &str) -> DispatchKind {
    match object_kind {
        "Page" => DispatchKind::PageRun,
        "Report" => DispatchKind::ReportRun,
        _ => DispatchKind::CodeunitRun,
    }
}

// ---------------------------------------------------------------------------
// Dependency classification (computed ONCE per resolve_calls).
// ---------------------------------------------------------------------------

/// A declared dependency (the L3-relevant subset of al-sem's ManifestDependency).
#[derive(Debug, Clone)]
pub struct DeclaredDependency {
    pub app_guid: String,
}

/// True when at least one declared dep's appGuid is absent from the fetched-app
/// set. Faithful port of `hasUnfetchedDeclaredDependency`. In the source-only
/// path `primary_dependencies` is empty → false.
fn has_unfetched_declared_dependency(
    primary_dependencies: &[DeclaredDependency],
    fetched_app_guids: &[String],
) -> bool {
    if primary_dependencies.is_empty() {
        return false;
    }
    let fetched: std::collections::HashSet<String> =
        fetched_app_guids.iter().map(|g| g.to_lowercase()).collect();
    primary_dependencies
        .iter()
        .any(|d| !fetched.contains(&d.app_guid.to_lowercase()))
}

// ---------------------------------------------------------------------------
// Interface dispatch.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn resolve_interface_dispatch(
    from: &str,
    callsite_id: &str,
    operation_id: &str,
    interface_name: &str,
    method_name: &str,
    caller: &L3Routine,
    call_site: &PCallSite,
    symbols: &SymbolTable,
    state: &mut BindingState,
) -> Vec<CallEdge> {
    let impls = symbols.objects_implementing(interface_name); // codeunits only, sorted by id
    let enum_impls = symbols.enum_implementers(interface_name);

    // Interface dispatch is polymorphic — bindings are ambiguous, never upgraded.
    mark_bindings_ambiguous(state);

    let mut resolved_edges: Vec<CallEdge> = Vec::new();
    let mut unresolved_impls: Vec<(String, String)> = Vec::new();

    for impl_obj in &impls {
        match resolve_by_name_and_arity(symbols, &impl_obj.id, method_name, caller, call_site) {
            ArityResolution::Resolved(r) => {
                let mut e = CallEdge::base(from, callsite_id, operation_id);
                e.to = Some(r.id.clone());
                e.dispatch_kind = DispatchKind::Interface;
                e.resolution = Resolution::Maybe;
                resolved_edges.push(e);
            }
            ArityResolution::NotFound => {
                unresolved_impls.push((impl_obj.id.clone(), "not-found".to_string()));
            }
            ArityResolution::NoArityMatch(_) => {
                unresolved_impls.push((impl_obj.id.clone(), "no-arity-match".to_string()));
            }
            ArityResolution::Ambiguous(_) => {
                unresolved_impls.push((impl_obj.id.clone(), "ambiguous".to_string()));
            }
        }
    }

    let enum_implementer_ids: Vec<String> = enum_impls.iter().map(|e| e.id.clone()).collect();

    let dispatch_meta = DispatchMeta {
        interface_name: interface_name.to_string(),
        total_impls: impls.len(),
        unresolved_impls,
        enum_implementers: enum_implementer_ids,
    };

    if resolved_edges.is_empty() {
        let mut e = CallEdge::base(from, callsite_id, operation_id);
        e.dispatch_kind = DispatchKind::Interface;
        e.resolution = Resolution::Unknown(UnknownReason::InterfaceNoImpl);
        e.dispatch_meta = Some(dispatch_meta);
        return vec![e];
    }

    // Sort resolved edges by `to` (byte-order on the internal routine id),
    // matching al-sem `(a.to ?? "") < (b.to ?? "")`.
    resolved_edges.sort_by(|a, b| {
        a.to.clone()
            .unwrap_or_default()
            .cmp(&b.to.clone().unwrap_or_default())
    });
    // dispatchMeta on the FIRST edge only.
    resolved_edges[0].dispatch_meta = Some(dispatch_meta);
    resolved_edges
}

// ---------------------------------------------------------------------------
// Per-callsite resolver.
// ---------------------------------------------------------------------------

fn resolve_call_site(
    routine: &L3Routine,
    call_site: &PCallSite,
    symbols: &SymbolTable,
    diagnostics: &mut Vec<Diagnostic>,
    unfetched_declared_dependency: bool,
    state: &mut BindingState,
) -> Vec<CallEdge> {
    let from = routine.id.as_str();
    let callsite_id = call_site.id.as_str();
    let operation_id = call_site.operation_id.as_str();

    use crate::engine::l2::features::PCallee;
    match &call_site.callee {
        PCallee::Bare { name } => {
            match resolve_by_name_and_arity(symbols, &routine.object_id, name, routine, call_site) {
                ArityResolution::Resolved(r) => {
                    if let Some(d) = upgrade_bindings(state, r, callsite_id) {
                        diagnostics.push(d);
                    }
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    e.to = Some(r.id.clone());
                    e.dispatch_kind = DispatchKind::Direct;
                    e.resolution = Resolution::Resolved;
                    vec![e]
                }
                ArityResolution::NotFound => {
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    if global_builtin_disposition(name).is_some() {
                        e.dispatch_kind = DispatchKind::Builtin;
                        e.resolution = Resolution::Builtin;
                    } else {
                        e.dispatch_kind = DispatchKind::Unresolved;
                        e.resolution = Resolution::Unknown(UnknownReason::BareUnresolved);
                    }
                    vec![e]
                }
                ArityResolution::NoArityMatch(candidates) => {
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    e.dispatch_kind = DispatchKind::Direct;
                    e.resolution = Resolution::MemberNotFound;
                    e.candidates = Some(sorted_ids(&candidates));
                    vec![e]
                }
                ArityResolution::Ambiguous(candidates) => {
                    mark_bindings_ambiguous(state);
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    e.dispatch_kind = DispatchKind::Direct;
                    e.resolution = Resolution::Ambiguous;
                    e.candidates = Some(sorted_ids(&candidates));
                    vec![e]
                }
            }
        }
        PCallee::ObjectRun {
            object_kind,
            target_type,
            target_ref,
            target_is_name,
        } => {
            let dispatch_kind = object_run_dispatch_kind(object_kind);
            let Some(target_ref) = target_ref else {
                // Dynamic target (a variable) — known shape, unknown target.
                let mut e = CallEdge::base(from, callsite_id, operation_id);
                e.dispatch_kind = DispatchKind::Dynamic;
                e.resolution = Resolution::Unknown(UnknownReason::DynamicObjectRunTarget);
                return vec![e];
            };
            let target_object = if *target_is_name {
                symbols.object_by_type_name(target_type, target_ref)
            } else {
                match target_ref.parse::<i64>() {
                    Ok(n) => symbols.object_by_type_number(target_type, n),
                    Err(_) => None,
                }
            };
            let Some(target_object) = target_object else {
                // Target named/numbered but not in indexed source.
                let mut e = CallEdge::base(from, callsite_id, operation_id);
                e.dispatch_kind = dispatch_kind;
                e.resolution = Resolution::Opaque;
                return vec![e];
            };
            // Entry routine: OnRun trigger, else the first routine in document order.
            let entry = symbols
                .routine_in_object(&target_object.id, "OnRun")
                .or_else(|| {
                    symbols
                        .routines_in_object(&target_object.id)
                        .into_iter()
                        .next()
                });
            if let Some(entry) = entry {
                if let Some(d) = upgrade_bindings(state, entry, callsite_id) {
                    diagnostics.push(d);
                }
                let mut e = CallEdge::base(from, callsite_id, operation_id);
                e.to = Some(entry.id.clone());
                e.dispatch_kind = dispatch_kind;
                e.resolution = Resolution::Resolved;
                return vec![e];
            }
            let mut e = CallEdge::base(from, callsite_id, operation_id);
            e.dispatch_kind = dispatch_kind;
            e.resolution = Resolution::Opaque;
            vec![e]
        }
        PCallee::Member { receiver, method } => {
            // Step 1 — simple receiver name.
            let Some(receiver_name) = simple_receiver_name(receiver) else {
                return unknown_method(
                    from,
                    callsite_id,
                    operation_id,
                    UnknownReason::CompoundReceiver,
                );
            };
            // Step 2 — find the receiver variable (params → locals → globals).
            let Some(recv_var) = routine.variables.iter().find(|v| v.name == receiver_name) else {
                return unknown_method(
                    from,
                    callsite_id,
                    operation_id,
                    UnknownReason::UntrackedReceiver,
                );
            };
            // Step 3 — parse the declared type into an object type reference.
            let Some(type_ref) = parse_object_type_ref(&recv_var.declared_type) else {
                // Phase 2: the receiver is a Record / RecordRef / FieldRef / KeyRef
                // / framework type (none of which `parse_object_type_ref` accepts).
                // If the method is a recognized COMPILER INTRINSIC, the edge is a
                // platform `builtin` terminal — NOT a resolution hole. A Record
                // method that is NOT an intrinsic (a real table procedure) stays
                // `unknown` here; table-procedure resolution is Phase 3.
                match super::member_builtins::classify_receiver(&recv_var.declared_type) {
                    Some(kind) => {
                        let method_lc = method.to_lowercase();
                        if super::member_builtins::member_builtin_disposition(kind, &method_lc)
                            .is_some()
                        {
                            // Both `Builtin` and `FlowsType` emit `builtin` in Phase 2.
                            // (FlowsType — RecordRef Open/GetTable/SetTable — is marked in
                            // the catalog for the §5 dynamic->static work, not yet emitted
                            // differently.)
                            let mut e = CallEdge::base(from, callsite_id, operation_id);
                            e.dispatch_kind = DispatchKind::Builtin;
                            e.resolution = Resolution::Builtin;
                            return vec![e];
                        }
                        // Catalog MISS: a Record receiver's non-builtin method is a
                        // real table procedure (Phase-3 resolvable); a framework
                        // receiver's miss is a catalog gap — capture the detail for
                        // the `aldump --l3-unknown-breakdown` diagnostic.
                        if kind == super::member_builtins::ReceiverBuiltinKind::Record {
                            return unknown_method(
                                from,
                                callsite_id,
                                operation_id,
                                UnknownReason::RecordTableProcedure,
                            );
                        } else {
                            let mut edges = unknown_method(
                                from,
                                callsite_id,
                                operation_id,
                                UnknownReason::FrameworkMethodNotInCatalog,
                            );
                            if let Some(e) = edges.first_mut() {
                                e.unknown_method_name = Some(format!("{:?}::{}", kind, method_lc));
                            }
                            return edges;
                        }
                    }
                    // Primitive / Variant / unrecognized declared type.
                    None => {
                        return unknown_method(
                            from,
                            callsite_id,
                            operation_id,
                            UnknownReason::NonObjectReceiverType,
                        );
                    }
                }
            };

            // Step 4 — interface dispatch.
            if type_ref.kind == ObjectKind::Interface {
                return resolve_interface_dispatch(
                    from,
                    callsite_id,
                    operation_id,
                    &type_ref.name,
                    method,
                    routine,
                    call_site,
                    symbols,
                    state,
                );
            }
            // Step 5 — enum statics are not callable methods.
            if type_ref.kind == ObjectKind::Enum {
                return unknown_method(from, callsite_id, operation_id, UnknownReason::EnumStatic);
            }
            // Step 6 — Codeunit / Page / Report / Query / XmlPort dispatch.
            let obj = symbols.object_by_type_name(type_ref.kind.as_str(), &type_ref.name);
            let Some(obj) = obj else {
                let external = ExternalTypeRef {
                    kind: type_ref.kind.as_str().to_string(),
                    name: type_ref.name.clone(),
                };
                let mut e = CallEdge::base(from, callsite_id, operation_id);
                e.dispatch_kind = DispatchKind::Method;
                e.external_type_ref = Some(external);
                e.resolution = if unfetched_declared_dependency {
                    Resolution::Opaque
                } else {
                    Resolution::ExternalTarget
                };
                return vec![e];
            };

            match resolve_by_name_and_arity(symbols, &obj.id, method, routine, call_site) {
                ArityResolution::Resolved(r) => {
                    if let Some(d) = upgrade_bindings(state, r, callsite_id) {
                        diagnostics.push(d);
                    }
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    e.to = Some(r.id.clone());
                    e.dispatch_kind = DispatchKind::Method;
                    e.resolution = Resolution::Resolved;
                    e.receiver_type = Some(recv_var.declared_type.clone());
                    vec![e]
                }
                ArityResolution::NotFound => {
                    // Built-in instance `<codeunitVar>.Run([Rec])` → OnRun trigger,
                    // when the codeunit has an OnRun and arity ≤ 1.
                    if type_ref.kind == ObjectKind::Codeunit
                        && method.to_lowercase() == "run"
                        && call_site.argument_bindings.len() <= 1
                    {
                        if let Some(on_run) = symbols.routine_in_object(&obj.id, "OnRun") {
                            if let Some(d) = upgrade_bindings(state, on_run, callsite_id) {
                                diagnostics.push(d);
                            }
                            let mut e = CallEdge::base(from, callsite_id, operation_id);
                            e.to = Some(on_run.id.clone());
                            e.dispatch_kind = DispatchKind::CodeunitRun;
                            e.resolution = Resolution::Resolved;
                            return vec![e];
                        }
                    }
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    e.dispatch_kind = DispatchKind::Method;
                    e.resolution = Resolution::MemberNotFound;
                    vec![e]
                }
                ArityResolution::NoArityMatch(candidates) => {
                    let ids = sorted_ids(&candidates);
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    e.dispatch_kind = DispatchKind::Method;
                    e.resolution = Resolution::MemberNotFound;
                    if !ids.is_empty() {
                        e.candidates = Some(ids);
                    }
                    vec![e]
                }
                ArityResolution::Ambiguous(candidates) => {
                    mark_bindings_ambiguous(state);
                    let mut e = CallEdge::base(from, callsite_id, operation_id);
                    e.dispatch_kind = DispatchKind::Method;
                    e.resolution = Resolution::Ambiguous;
                    e.candidates = Some(sorted_ids(&candidates));
                    vec![e]
                }
            }
        }
        PCallee::Unknown => {
            let mut e = CallEdge::base(from, callsite_id, operation_id);
            e.dispatch_kind = DispatchKind::Unresolved;
            e.resolution = Resolution::Unknown(UnknownReason::CalleeUnknown);
            vec![e]
        }
    }
}

fn unknown_method(
    from: &str,
    callsite_id: &str,
    operation_id: &str,
    reason: UnknownReason,
) -> Vec<CallEdge> {
    let mut e = CallEdge::base(from, callsite_id, operation_id);
    e.dispatch_kind = DispatchKind::Method;
    e.resolution = Resolution::Unknown(reason);
    vec![e]
}

/// `routines.map(r => r.id).sort()` — byte-order sort of internal routine ids.
fn sorted_ids(routines: &[&L3Routine]) -> Vec<String> {
    let mut ids: Vec<String> = routines.iter().map(|r| r.id.clone()).collect();
    ids.sort();
    ids
}

// ---------------------------------------------------------------------------
// Top-level resolve.
// ---------------------------------------------------------------------------

/// The full call-resolution result: every edge + the per-callsite upgraded
/// bindings (keyed by internal callsite id) + diagnostics.
pub struct ResolvedCalls {
    pub edges: Vec<CallEdge>,
    /// internal callsite id → upgraded argument bindings (in argument order).
    pub upgraded_bindings: HashMap<String, Vec<UpgradedBinding>>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Resolve every call site in the workspace into CallEdges (+ implicit-trigger
/// edges), upgrading argument bindings exactly once per callsite. Faithful port
/// of `resolveCalls` + the merge of `buildImplicitTriggerEdges`.
///
/// `primary_dependencies` / `fetched_app_guids` feed the one-time
/// `has_unfetched_declared_dependency` evaluation. In the source-only path pass
/// empty slices → the boolean is false.
pub fn resolve_calls(
    workspace: &L3Workspace,
    symbols: &SymbolTable,
    primary_dependencies: &[DeclaredDependency],
    fetched_app_guids: &[String],
) -> ResolvedCalls {
    let mut edges: Vec<CallEdge> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut upgraded_bindings: HashMap<String, Vec<UpgradedBinding>> = HashMap::new();

    // Evaluate the unfetched-dep boolean ONCE.
    let unfetched = has_unfetched_declared_dependency(primary_dependencies, fetched_app_guids);

    for routine in &workspace.routines {
        for call_site in &routine.call_sites {
            let mut state = initial_binding_state(call_site);
            let result = resolve_call_site(
                routine,
                call_site,
                symbols,
                &mut diagnostics,
                unfetched,
                &mut state,
            );
            for e in result {
                edges.push(e);
            }
            // Capture the (possibly upgraded) bindings for this callsite. Only
            // callsites with ≥1 binding are meaningful; store all so the dump can
            // decide whether to emit.
            upgraded_bindings.insert(call_site.id.clone(), state.bindings);
        }
    }

    // Merge implicit-trigger edges (read-only; same internal-id shape).
    let implicit = build_implicit_trigger_edges(workspace, symbols);
    for e in implicit {
        edges.push(e);
    }

    ResolvedCalls {
        edges,
        upgraded_bindings,
        diagnostics,
    }
}
