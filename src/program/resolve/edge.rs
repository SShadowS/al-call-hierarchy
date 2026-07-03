//! The multi-axis behaviour-edge model + the obligation-based real-unknown metric.
//! Spec §3 / §3.2.
//!
//! # Reachability contract
//!
//! Routes stored in `Edge.routes` encode ALL possible dispatch targets, including
//! routes that only fire when the caller explicitly calls `BindSubscription` at
//! runtime (`Condition::ManualBinding`).  Code that builds a "who actually fires by
//! default" reachability set **MUST** use [`Edge::default_reachable_routes`] (only
//! unconditionally-bound routes) and **MUST NOT** traverse `ManualBinding` routes as
//! unconditional edges.
//!
//! Use [`Edge::may_reachable_routes`] for opt-in "could this fire" queries (includes
//! `ManualBinding`).  Use [`Edge::all_routes`] exclusively for resolution/gate/
//! classification context (classify_obligation, differential projection).
//!
//! The `routes` field itself is kept `pub` to allow struct-literal construction
//! across the crate.  The named accessors are the enforced API for consumers.

use crate::program::node::{AppRef, RoutineNodeId};
use crate::snapshot::TrustTier;

/// Caller / target identity is a 1B.1 app-qualified routine node.
pub type NodeId = RoutineNodeId;

/// The kind of an ABI-boundary routine for routing and auditability.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AbiRoutineKind {
    Procedure,
    EventPublisher,
    EventSubscriber,
}

/// The event classification for an ABI event publisher.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AbiEventKind {
    None,
    Integration,
    Business,
    Internal,
}

/// Structured, stable identity of an ABI-boundary routine.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiRoutineKey {
    pub app: AppRef,
    pub object_type: String,
    pub object_number: i64,
    pub object_name_lc: String,
    pub routine_name_lc: String,
    pub params_count: usize,
    pub param_type_fp: u64,
    pub routine_kind: AbiRoutineKind,
    pub event_kind: AbiEventKind,
}

/// A platform builtin's catalog identity (clean-room catalog id; Phase 2+).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuiltinId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourcePos {
    pub line: u32,
    pub col: u32,
}

/// Line/col span in a named source unit — the coordinate BOTH engines align on
/// (L3 records line/col via `PAnchor`; the fresh side converts IR byte-origins).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalSpan {
    pub unit: String,
    pub start: SourcePos,
    pub end: SourcePos,
}

/// Stable SEMANTIC identity of an originating site (spec §6.1) — span-based, never positional.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SiteId {
    pub caller: NodeId,
    pub span: CanonicalSpan,
    pub callee_fingerprint: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    Call,
    Run,
    ImplicitTrigger,
    EventFlow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DispatchShape {
    Exact,
    Polymorphic,
    Multicast,
    DynamicOpen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OpenWorldReason {
    ReverseDependentImplementers,
    ReverseDependentSubscribers,
    ReverseDependentExtensions,
    RuntimeTypeUnbounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SetCompleteness {
    /// Provably exhaustive (sealed / closed-world snapshot) — NOT merely "enumerated the snapshot".
    Complete,
    /// Open world may add routes; also the edge-level home of a DynamicOpen blocker
    /// (`RuntimeTypeUnbounded`) and of a legal empty fan-out.
    Partial { reason: OpenWorldReason },
}

/// Diagnostic reason for an [`Evidence::Unknown`] route (Task 3; charter §8
/// stratified reporting).
///
/// Fresh-native: carries NO information from `engine::l3`/`engine::l2` (the
/// grep-guard on `src/program/resolve` importing those modules stays green).
/// Purely a DIAGNOSTIC payload — it never feeds [`classify_obligation`] or
/// [`ObligationOutcome`]; the real-`unknown` COUNT and classification are
/// byte-identical with or without this enum's existence. It exists so the
/// ~2% residual `unknown` edge rate can be precisely characterized (which of
/// the ~13 structurally-distinct decline sites produced each edge) instead of
/// collapsing into one bare, uninformative bucket.
///
/// `derive(Ord)` gives a deterministic `BTreeMap<UnknownReason, usize>`
/// iteration order for `aldump`'s stratified breakdown. Render via
/// [`UnknownReason::as_str`], never `Debug` — `Debug`'s PascalCase spelling is
/// not a public wire format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnknownReason {
    /// `CalleeShape::Unknown` whose raw callee text is a multi-segment
    /// (`A.B.C`, ≥2 dots) receiver chain the extractor could not classify
    /// into a `Member` call.
    CompoundReceiver,
    /// The receiver's static type could not be tracked at all:
    /// `ReceiverType::Unknown`/`Dynamic` (a runtime-typed Variant receiver),
    /// or an `ObjectRun` target that is a runtime variable rather than a
    /// static name/number.
    UntrackedReceiver,
    /// `CalleeShape::Unknown` for any other call expression shape the
    /// extractor could not classify (not multi-segment — see
    /// [`Self::CompoundReceiver`]).
    UnclassifiedCallee,
    /// GENUINE overload ambiguity ONLY (reason-split Task 2 — narrowed from
    /// its pre-Task-2 meaning, which also covered [`Self::ArityMismatch`],
    /// [`Self::AbiCollapsedOverload`], and [`Self::AccessFilteredOverload`]
    /// below): `>1` visible, arity-matched, DISTINCT `RoutineNodeId`
    /// candidates this engine cannot break by name+arity+visibility alone —
    /// the textbook case (e.g. two real 2-arg source overloads). Also used
    /// by table-scope/interface/trigger-fan-out sites structurally identical
    /// to this shape but outside `resolve_in_object` (unchanged by Task 2).
    OverloadAmbiguous,
    /// A name was found in the candidate object, but ZERO overloads match
    /// the call's arity (`resolve_in_object`'s `pre_filter_count == 0`) —
    /// nothing to be ambiguous BETWEEN; distinct from [`Self::OverloadAmbiguous`]
    /// (reason-split Task 2).
    ArityMismatch,
    /// The sole arity-matched, visible candidate is [`RoutineNode::
    /// abi_overload_collapsed`]-marked: an ABI ingestion-fidelity admission
    /// (≥2 raw ABI entries fingerprint-collided into one arbitrary/
    /// indistinguishable survivor), NOT a live candidate-set ambiguity
    /// (reason-split Task 2; `resolve_in_object`'s PLAIN-DISPATCH MARKER
    /// GUARD only — the other `routine_is_collapse_marked` call sites
    /// outside `resolve_in_object` are unchanged by Task 2 and still emit
    /// [`Self::OverloadAmbiguous`]).
    AbiCollapsedOverload,
    /// Access filtering narrowed an originally-ambiguous (`pre_filter_count
    /// > 1`) same-arity candidate set down to exactly ONE visible survivor,
    /// and the resolver declined rather than select it (the pre-filter set
    /// was ambiguous with no arg-type evidence to pick between overloads, so
    /// access removing the other sibling(s) doesn't prove the call meant the
    /// survivor) — a distinct diagnostic shape from a genuinely >1-visible
    /// ambiguity (reason-split Task 2; `resolve_in_object` only).
    AccessFilteredOverload,
    /// A bare-call table-scope candidate collides in name+arity with a
    /// global builtin or a bare-callable page/instance intrinsic — unproven
    /// precedence, fail closed rather than guess which wins.
    BuiltinPrecedenceCollision,
    /// A bare call lexically inside a `with` block (or whose `with`-freedom
    /// could not be proven) — implicit-`Rec` dispatch (Step 3) is skipped
    /// unconditionally rather than risk a false `Source` inside an
    /// unrepresented `with`.
    WithScopeGuard,
    /// A bare call from a `Codeunit`: implicit-`Rec` dispatch is a Page/Table
    /// source-record mechanism only — a `Codeunit`'s `TableNo` is never
    /// consulted by Step 3.
    CodeunitTableNoExcluded,
    /// A bare/record-op call from a `Report`/`ReportExtension`: the
    /// per-dataitem implicit `Rec` is not object-level and is not modeled.
    ReportRecExcluded,
    /// A same-name/arity candidate exists but its declared `Protected`
    /// access is not visible from the caller's identity.
    ProtectedNotVisible,
    /// A same-name/arity candidate exists but its declared `Local` access is
    /// not visible outside its declaring object.
    LocalNotVisible,
    /// A same-name/arity candidate exists but its declared `Internal` access
    /// is not visible outside its declaring app.
    InternalNotVisible,
    /// The relevant platform builtin catalog (Record / RecordRef / FieldRef /
    /// KeyRef / Framework / Enum) has no entry for this method name.
    CatalogMiss,
    /// No unique in-closure receiver/table identity: an ambiguous cross-app
    /// name, an out-of-closure declared type, or an otherwise-unresolved
    /// receiver.
    ReceiverOutOfClosure,
    /// MEMBER-absent-on-a-RESOLVED-surface ONLY (reason-split Task 2 —
    /// narrowed from its pre-Task-2 meaning, which also covered
    /// [`Self::ObjectNotInGraph`] below): the RECEIVER object was resolved
    /// (own object, extension base, target object, or interface implementer
    /// — all found in the graph), but the callee name is not declared
    /// anywhere reachable from it — genuine absence, not a visibility or
    /// overload issue. Pairs with [`Route::receiver_tier`] (populated at
    /// every `MemberNotFound` emission site): only a source-complete tier
    /// (`Workspace`/`EmbeddedSource`/`LocalSourceVerified`/
    /// `LocalSourceApproximate`) can ever PROVE a member's absence —
    /// `SymbolOnly`'s ABI listing is not exhaustive of the real object, so a
    /// `SymbolOnly`-tagged `MemberNotFound` is honest-but-unprovable, never a
    /// stronger claim.
    MemberNotFound,
    /// The RECEIVER OBJECT itself is absent from the whole-program graph —
    /// not in workspace source, not in any dependency's SymbolReference
    /// (reason-split Task 2, split out of the old `MemberNotFound`). Makes
    /// NO externality claim (an `UndeclaredExternalTarget`-style label was
    /// considered and dropped: externality is unprovable from mere absence —
    /// name prefixes/sampling/not-in-graph are all disallowed proofs per the
    /// charter's open-world discipline). `receiver_tier` is intentionally
    /// left `None` here — there is no resolved receiver to tag.
    ObjectNotInGraph,
    /// An internal index/body-map lookup that should structurally never miss
    /// did — a defensive fallback, not a normal AL-semantics decline.
    IndexIntegrationGap,
}

impl UnknownReason {
    /// Stable camelCase identifier for diagnostic rendering (`aldump`'s
    /// stratified breakdown). Render via this, NEVER `Debug`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            UnknownReason::CompoundReceiver => "compoundReceiver",
            UnknownReason::UntrackedReceiver => "untrackedReceiver",
            UnknownReason::UnclassifiedCallee => "unclassifiedCallee",
            UnknownReason::OverloadAmbiguous => "overloadAmbiguous",
            UnknownReason::ArityMismatch => "arityMismatch",
            UnknownReason::AbiCollapsedOverload => "abiCollapsedOverload",
            UnknownReason::AccessFilteredOverload => "accessFilteredOverload",
            UnknownReason::BuiltinPrecedenceCollision => "builtinPrecedenceCollision",
            UnknownReason::WithScopeGuard => "withScopeGuard",
            UnknownReason::CodeunitTableNoExcluded => "codeunitTableNoExcluded",
            UnknownReason::ReportRecExcluded => "reportRecExcluded",
            UnknownReason::ProtectedNotVisible => "protectedNotVisible",
            UnknownReason::LocalNotVisible => "localNotVisible",
            UnknownReason::InternalNotVisible => "internalNotVisible",
            UnknownReason::CatalogMiss => "catalogMiss",
            UnknownReason::ReceiverOutOfClosure => "receiverOutOfClosure",
            UnknownReason::MemberNotFound => "memberNotFound",
            UnknownReason::ObjectNotInGraph => "objectNotInGraph",
            UnknownReason::IndexIntegrationGap => "indexIntegrationGap",
        }
    }
}

impl std::fmt::Display for UnknownReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Evidence {
    Source,
    Abi,
    Catalog,
    /// ABI body-unavailable boundary ONLY — never a visibility conclusion (spec §5.4).
    Opaque,
    /// Genuine resolution failure, carrying a diagnostic [`UnknownReason`]
    /// (Task 3). The payload is REQUIRED at construction (no zero-arg
    /// constructor survives — see [`crate::program::resolve::resolver`]'s
    /// `member_unknown_route`/`unresolved_route`), which is what forces every
    /// decline site in the resolver to be tagged: the compiler enumerates
    /// every construction/match on this variant.
    ///
    /// **Serialization boundary (Task 3):** the payload MUST NEVER be
    /// serialized into or compared against the committed semantic goldens
    /// (`tests/goldens/semantic-edges/*.json`) or the semantic-audit path —
    /// those use [`Evidence::kind`], which projects `Unknown(_)` down to the
    /// same [`EvidenceKind::Unknown`] regardless of reason. The reason lives
    /// ONLY in the `aldump --program-call-graph-stats` `unknownByReason`
    /// diagnostic breakdown.
    Unknown(UnknownReason),
}

/// Serialization/comparison-stable PROJECTION of [`Evidence`] that discards
/// the [`UnknownReason`] payload — `Unknown(_)` always maps to the same
/// `Unknown` kind, regardless of reason. Every semantic-golden /
/// semantic-audit serialization and comparison path MUST use
/// [`Evidence::kind`], never the raw `Evidence` value, so a future change to
/// `UnknownReason` can never perturb the committed anonymized goldens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EvidenceKind {
    Source,
    Abi,
    Catalog,
    Opaque,
    Unknown,
}

impl Evidence {
    /// Project away the [`UnknownReason`] payload — see [`EvidenceKind`].
    #[must_use]
    pub fn kind(&self) -> EvidenceKind {
        match self {
            Evidence::Source => EvidenceKind::Source,
            Evidence::Abi => EvidenceKind::Abi,
            Evidence::Catalog => EvidenceKind::Catalog,
            Evidence::Opaque => EvidenceKind::Opaque,
            Evidence::Unknown(_) => EvidenceKind::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Condition {
    RunTriggerGuarded,
    /// The subscriber fires ONLY when `BindSubscription` is called explicitly at runtime.
    /// Routes with this condition do NOT fire by default and MUST be excluded from
    /// default reachability traversal. Use [`Route::fires_by_default`].
    ManualBinding,
    /// The subscriber fires by default but may be skipped at runtime when the
    /// required license is absent.  Treated as fires-by-default for reachability.
    SkipOnMissingLicense,
    /// As `SkipOnMissingLicense` but for permission checks.
    SkipOnMissingPermission,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RouteTarget {
    Routine(NodeId),
    Builtin(BuiltinId),
    /// Known public boundary whose body is unavailable — retains structured identity.
    AbiSymbol {
        key: AbiRoutineKey,
    },
    /// Genuine failure only — pairs with `Evidence::Unknown`.
    Unresolved,
}

/// Independent-checkability handle for a route's evidence (spec §5.5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Witness {
    SourceSpan {
        file: String,
        span: (u32, u32),
    },
    AbiSymbol {
        key: AbiRoutineKey,
    },
    CatalogEntry {
        id: BuiltinId,
        catalog_version: String,
    },
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Route {
    pub target: RouteTarget,
    pub evidence: Evidence,
    /// Zero or more dispatch conditions on this route.  Empty means the route fires
    /// unconditionally.  See [`Condition`] and [`Route::fires_by_default`].
    pub conditions: Vec<Condition>,
    pub witness: Witness,
    /// Diagnostic-only, additive field (reason-split Task 2): the resolved
    /// RECEIVER object's [`TrustTier`], populated ONLY alongside
    /// `Evidence::Unknown(UnknownReason::MemberNotFound)` routes — the
    /// member-absent-on-a-resolved-surface shape, where a receiver object
    /// WAS found so its tier is knowable. `None` everywhere else, INCLUDING
    /// `UnknownReason::ObjectNotInGraph` (no resolved receiver exists there
    /// to tag). NOT a reason split — `MemberNotFound` stays one stable
    /// `as_str()` key; consumers group by `(reason, receiver_tier)` (see
    /// [`unknown_receiver_tier_breakdown`]). NEVER consulted by
    /// `classify_obligation`/`ObligationOutcome`, and NEVER compared against
    /// the committed semantic goldens (same serialization-boundary discipline
    /// as [`Evidence::Unknown`]'s payload — see [`Evidence::kind`]).
    pub receiver_tier: Option<TrustTier>,
}

impl Route {
    /// Returns `true` when this route fires by default (no explicit binding required).
    ///
    /// Returns `false` iff `conditions` contains [`Condition::ManualBinding`] — meaning
    /// the subscriber fires only when `BindSubscription` is explicitly called at runtime.
    ///
    /// `SkipOnMissingLicense` and `SkipOnMissingPermission` fire by default (they are
    /// bound; they may runtime-skip but are NOT deferred by a caller binding step).
    /// Only `ManualBinding` causes the route to not fire by default.
    pub fn fires_by_default(&self) -> bool {
        !self.conditions.contains(&Condition::ManualBinding)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge {
    pub from: NodeId,
    pub site: SiteId,
    pub kind: EdgeKind,
    pub shape: DispatchShape,
    pub completeness: SetCompleteness,
    pub routes: Vec<Route>,
}

impl Edge {
    /// All routes on this edge, for **RESOLUTION context** only — gate checks,
    /// `classify_obligation`, and canonical differential projection.
    ///
    /// Reachability consumers MUST use [`default_reachable_routes`][Self::default_reachable_routes]
    /// or [`may_reachable_routes`][Self::may_reachable_routes] instead.
    pub fn all_routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter()
    }

    /// Routes that fire by default — excludes [`Condition::ManualBinding`] routes.
    ///
    /// Use for default reachability traversal: "what fires without any explicit
    /// caller action?"  A `ManualBinding` subscriber does NOT fire unless
    /// `BindSubscription` is called; including it in unconditional traversal would
    /// overstate reachability.
    pub fn default_reachable_routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter().filter(|r| r.fires_by_default())
    }

    /// All routes that **could** fire, including those requiring explicit
    /// `BindSubscription` (`ManualBinding`).
    ///
    /// Use for opt-in / "may reach" reachability queries.
    pub fn may_reachable_routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObligationOutcome {
    /// ≥1 real route fires by default (unconditional obligation met).
    Resolved,
    /// All real routes require explicit `BindSubscription` (`ManualBinding`).
    ///
    /// Treated as **resolved-for-resolution** (not a gap; the engine found the
    /// targets) but **distinct from `Resolved`** so reachability consumers can
    /// choose whether to traverse these edges.  `real_unknown_rate` does NOT count
    /// this as Unknown.  Use [`Edge::default_reachable_routes`] to exclude these
    /// from unconditional traversal; [`Edge::may_reachable_routes`] to include them.
    ConditionalResolved,
    HonestDynamic,
    HonestEmpty,
    Unknown,
}

/// Classify one edge's resolution obligation (spec §3.2).
pub fn classify_obligation(e: &Edge) -> ObligationOutcome {
    // Collect real routes: non-Unknown evidence AND non-Unresolved target.
    let mut has_real = false;
    let mut all_manual = true; // only meaningful when has_real is true

    for r in &e.routes {
        if r.evidence.kind() != EvidenceKind::Unknown && r.target != RouteTarget::Unresolved {
            has_real = true;
            if r.fires_by_default() {
                all_manual = false;
            }
        }
    }

    if has_real {
        return if all_manual {
            // All real routes require explicit BindSubscription — conditional obligation.
            ObligationOutcome::ConditionalResolved
        } else {
            // ≥1 real route fires by default — unconditional obligation met.
            ObligationOutcome::Resolved
        };
    }

    if e.shape == DispatchShape::DynamicOpen {
        return ObligationOutcome::HonestDynamic;
    }
    let is_fanout = matches!(
        e.shape,
        DispatchShape::Polymorphic | DispatchShape::Multicast
    );
    let is_open = matches!(e.completeness, SetCompleteness::Partial { .. });
    if e.routes.is_empty() && is_fanout && is_open {
        return ObligationOutcome::HonestEmpty;
    }
    ObligationOutcome::Unknown
}

/// real-unknown = Unknown obligations / all obligations (spec §3.2).
///
/// `ConditionalResolved` edges count as resolved-for-resolution (not Unknown).
pub fn real_unknown_rate(edges: &[Edge]) -> f64 {
    if edges.is_empty() {
        return 0.0;
    }
    let unknown = edges
        .iter()
        .filter(|e| classify_obligation(e) == ObligationOutcome::Unknown)
        .count();
    unknown as f64 / edges.len() as f64
}

/// Deterministic (within a process run) fingerprint of a callee's text, used as
/// part of a call site's identity. BOTH the fresh and L3 projections must use THIS
/// function so the differential cannot drift.
pub(crate) fn callee_fp(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.to_ascii_lowercase().hash(&mut h);
    h.finish()
}

/// Stratified counts for `--program-call-graph-stats`.
///
/// `resolved` has been split into three sub-counts by evidence so that the
/// contribution of ABI ingestion is visible without laundering external
/// boundaries into the "resolved" bucket.  `real_unknown_rate` (= `unknown /
/// total`) is unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Histogram {
    pub total: usize,
    /// Edges resolved with in-source evidence (`Evidence::Source`).
    pub resolved_source: usize,
    /// Edges resolved via the builtin catalog (`Evidence::Catalog`).
    pub resolved_catalog: usize,
    /// Edges resolved to an ABI-boundary symbol (`Evidence::Abi` or
    /// `Evidence::Opaque` — the callee is in a SymbolOnly dependency).
    /// The real-unknown-rate DROP from ABI ingestion shows here, NOT in
    /// `resolved_source` — no laundering of external boundaries.
    pub resolved_abi_external: usize,
    /// Edges where all real routes require explicit `BindSubscription`.
    /// Counted as resolved-for-resolution (not unknown).
    pub conditional_resolved: usize,
    pub honest_dynamic: usize,
    pub honest_empty: usize,
    pub unknown: usize,
}

impl Histogram {
    pub fn of_edges(edges: &[Edge]) -> Histogram {
        let mut h = Histogram::default();
        for e in edges {
            h.total += 1;
            match classify_obligation(e) {
                ObligationOutcome::Resolved => {
                    // Classify by the best evidence among all real default-firing
                    // routes.  Priority: Source (0) > Catalog (1) > Abi/Opaque (2).
                    let mut best: Option<u8> = None;
                    for r in &e.routes {
                        if r.evidence.kind() == EvidenceKind::Unknown
                            || r.target == RouteTarget::Unresolved
                            || !r.fires_by_default()
                        {
                            continue;
                        }
                        let score: u8 = match r.evidence {
                            Evidence::Source => 0,
                            Evidence::Catalog => 1,
                            Evidence::Abi | Evidence::Opaque => 2,
                            Evidence::Unknown(_) => continue,
                        };
                        best = Some(best.map_or(score, |b: u8| b.min(score)));
                    }
                    match best {
                        Some(0) => h.resolved_source += 1,
                        Some(1) => h.resolved_catalog += 1,
                        // Some(2): Abi/Opaque evidence (external dep ABI boundary).
                        Some(_) => h.resolved_abi_external += 1,
                        // None is unreachable for a Resolved edge: the obligation
                        // gate ensures at least one default-firing route with
                        // non-Unknown evidence.  If this fires, a new Evidence
                        // variant or a logic change broke the invariant.
                        None => unreachable!(
                            "Resolved edge must have >=1 default-firing non-Unknown route"
                        ),
                    }
                }
                ObligationOutcome::ConditionalResolved => h.conditional_resolved += 1,
                ObligationOutcome::HonestDynamic => h.honest_dynamic += 1,
                ObligationOutcome::HonestEmpty => h.honest_empty += 1,
                ObligationOutcome::Unknown => h.unknown += 1,
            }
        }
        h
    }

    pub fn real_unknown_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.unknown as f64 / self.total as f64
        }
    }
}

/// Stratified breakdown of `Unknown`-obligation edges by [`UnknownReason`]
/// (Task 3; charter §8 stratified reporting). Deterministic
/// (`BTreeMap` iteration order, [`UnknownReason`]'s derived `Ord`).
///
/// Counts per EDGE (mirrors [`Histogram`], which also counts edges, not
/// routes): an `Unknown`-classified edge structurally carries exactly one
/// `Unresolved`/`Unknown(reason)` route (every decline site in the resolver
/// returns a single-route `Vec`, never an empty one, for the non-fan-out
/// shapes that can reach `ObligationOutcome::Unknown` — see
/// `classify_obligation`), so the first `Unknown`-evidence route's reason is
/// used. `sum(values()) == ` the number of edges classified
/// [`ObligationOutcome::Unknown`] in `edges` — see the `unknown_reason_
/// breakdown_sum_matches_unknown_count` test below and the `aldump`
/// `--program-call-graph-stats` `unknownByReason` field.
///
/// DIAGNOSTIC ONLY: does not affect `classify_obligation`/`ObligationOutcome`
/// — the real-`unknown` count and classification are unchanged by this
/// function's existence.
#[must_use]
pub fn unknown_reason_breakdown<'a>(
    edges: impl IntoIterator<Item = &'a Edge>,
) -> std::collections::BTreeMap<UnknownReason, usize> {
    let mut map = std::collections::BTreeMap::new();
    for e in edges {
        if classify_obligation(e) != ObligationOutcome::Unknown {
            continue;
        }
        let reason = e.routes.iter().find_map(|r| match r.evidence {
            Evidence::Unknown(reason) => Some(reason),
            _ => None,
        });
        if let Some(reason) = reason {
            *map.entry(reason).or_insert(0) += 1;
        }
    }
    map
}

/// Stratified `(UnknownReason, receiver_tier)` breakdown (reason-split Task
/// 2). A SEPARATE function, not a change to [`unknown_reason_breakdown`]'s
/// signature — `receiver_tier` is an ADDITIVE diagnostic, not a reason split
/// (see [`Route::receiver_tier`]'s doc): today only `MemberNotFound` routes
/// ever carry `Some(tier)`; every other reason's routes report `None`, and
/// `sum(values()) == unknown_reason_breakdown(edges).values().sum()` (same
/// per-edge counting rule: one Unknown-classified edge contributes its first
/// `Unknown`-evidence route's `(reason, tier)` pair).
#[must_use]
pub fn unknown_receiver_tier_breakdown<'a>(
    edges: impl IntoIterator<Item = &'a Edge>,
) -> std::collections::BTreeMap<(UnknownReason, Option<TrustTier>), usize> {
    let mut map = std::collections::BTreeMap::new();
    for e in edges {
        if classify_obligation(e) != ObligationOutcome::Unknown {
            continue;
        }
        let hit = e.routes.iter().find_map(|r| match r.evidence {
            Evidence::Unknown(reason) => Some((reason, r.receiver_tier)),
            _ => None,
        });
        if let Some(key) = hit {
            *map.entry(key).or_insert(0) += 1;
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::{AppRef, ObjKey, ObjectKind, ObjectNodeId, RoutineNodeId};

    fn rid(app: u32, name: &str) -> RoutineNodeId {
        RoutineNodeId {
            object: ObjectNodeId {
                app: AppRef(app),
                kind: ObjectKind::Codeunit,
                key: ObjKey::Id(1),
            },
            name_lc: name.to_string(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        }
    }

    #[test]
    fn edge_constructs_and_is_orderable() {
        let e = Edge {
            from: rid(0, "post"),
            site: SiteId {
                caller: rid(0, "post"),
                span: CanonicalSpan {
                    unit: "u".into(),
                    start: SourcePos { line: 1, col: 1 },
                    end: SourcePos { line: 1, col: 9 },
                },
                callee_fingerprint: 42,
            },
            kind: EdgeKind::Call,
            shape: DispatchShape::Exact,
            completeness: SetCompleteness::Complete,
            routes: vec![Route {
                target: RouteTarget::Routine(rid(0, "helper")),
                evidence: Evidence::Source,
                conditions: vec![],
                witness: Witness::SourceSpan {
                    file: "f.al".into(),
                    span: (10, 20),
                },
                receiver_tier: None,
            }],
        };
        assert_eq!(e.routes.len(), 1);
        // Hashable + comparable (needed by the differential).
        let mut v = vec![e.clone(), e];
        v.sort();
        assert_eq!(v.len(), 2);
    }

    fn edge_with(
        kind: EdgeKind,
        shape: DispatchShape,
        comp: SetCompleteness,
        routes: Vec<Route>,
    ) -> Edge {
        Edge {
            from: rid(0, "c"),
            site: SiteId {
                caller: rid(0, "c"),
                span: CanonicalSpan {
                    unit: "u".into(),
                    start: SourcePos { line: 1, col: 1 },
                    end: SourcePos { line: 1, col: 2 },
                },
                callee_fingerprint: 1,
            },
            kind,
            shape,
            completeness: comp,
            routes,
        }
    }

    fn src_route() -> Route {
        Route {
            target: RouteTarget::Routine(rid(0, "t")),
            evidence: Evidence::Source,
            conditions: vec![],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        }
    }

    #[test]
    fn obligation_outcomes_are_correct() {
        // Resolved: >=1 non-Unknown route.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![src_route()]
            )),
            ObligationOutcome::Resolved
        );
        // HonestDynamic: DynamicOpen.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::Run,
                DispatchShape::DynamicOpen,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::RuntimeTypeUnbounded
                },
                vec![]
            )),
            ObligationOutcome::HonestDynamic
        );
        // HonestEmpty: fan-out, zero routes, Partial.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::EventFlow,
                DispatchShape::Multicast,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::ReverseDependentSubscribers
                },
                vec![]
            )),
            ObligationOutcome::HonestEmpty
        );
        // Unknown: Exact Call with no target.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![]
            )),
            ObligationOutcome::Unknown
        );
        // Metric: 1 Unknown out of 4 obligations.
        let edges = vec![
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![src_route()],
            ),
            edge_with(
                EdgeKind::Run,
                DispatchShape::DynamicOpen,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::RuntimeTypeUnbounded,
                },
                vec![],
            ),
            edge_with(
                EdgeKind::EventFlow,
                DispatchShape::Multicast,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::ReverseDependentSubscribers,
                },
                vec![],
            ),
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![],
            ),
        ];
        assert!((real_unknown_rate(&edges) - 0.25).abs() < 1e-9);
    }

    // ---- Phase 4b Task 0: conditions Vec + fires_by_default + ConditionalResolved ----

    fn manual_route() -> Route {
        Route {
            target: RouteTarget::Routine(rid(0, "manual_sub")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        }
    }

    fn skip_license_route() -> Route {
        Route {
            target: RouteTarget::Routine(rid(0, "skip_sub")),
            evidence: Evidence::Source,
            conditions: vec![Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (2, 3),
            },
            receiver_tier: None,
        }
    }

    #[test]
    fn route_conditions_vec_holds_multiple() {
        let r = Route {
            target: RouteTarget::Routine(rid(0, "t")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding, Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        };
        assert_eq!(r.conditions.len(), 2);
        assert!(r.conditions.contains(&Condition::ManualBinding));
        assert!(r.conditions.contains(&Condition::SkipOnMissingLicense));
    }

    #[test]
    fn fires_by_default_semantics() {
        // empty conditions → fires by default
        let r_empty = Route {
            target: RouteTarget::Routine(rid(0, "a")),
            evidence: Evidence::Source,
            conditions: vec![],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        };
        assert!(
            r_empty.fires_by_default(),
            "empty conditions must fire by default"
        );

        // ManualBinding → does NOT fire by default
        let r_manual = Route {
            target: RouteTarget::Routine(rid(0, "b")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        };
        assert!(
            !r_manual.fires_by_default(),
            "ManualBinding must NOT fire by default"
        );

        // SkipOnMissingLicense alone → fires by default (may runtime-skip but is bound)
        let r_skip = Route {
            target: RouteTarget::Routine(rid(0, "c")),
            evidence: Evidence::Source,
            conditions: vec![Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        };
        assert!(
            r_skip.fires_by_default(),
            "SkipOnMissingLicense must fire by default"
        );

        // [ManualBinding, SkipOnMissingLicense] → ManualBinding dominates → false
        let r_both = Route {
            target: RouteTarget::Routine(rid(0, "d")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding, Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        };
        assert!(
            !r_both.fires_by_default(),
            "[ManualBinding, Skip*] must NOT fire by default"
        );
    }

    #[test]
    fn conditional_resolved_vs_resolved_vs_empty() {
        // Edge whose ONLY real route is Manual → ConditionalResolved (NOT plain Resolved)
        let manual_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Complete,
            vec![manual_route()],
        );
        assert_eq!(
            classify_obligation(&manual_edge),
            ObligationOutcome::ConditionalResolved,
            "all-Manual-route edge must classify as ConditionalResolved, not Resolved"
        );

        // Edge with a fires-by-default real route → Resolved
        let default_edge = edge_with(
            EdgeKind::Call,
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![src_route()],
        );
        assert_eq!(
            classify_obligation(&default_edge),
            ObligationOutcome::Resolved,
            "fires-by-default route must classify as Resolved"
        );

        // Mixed: one Manual + one fires-by-default → Resolved (≥1 default-firing route)
        let mixed_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![manual_route(), src_route()],
        );
        assert_eq!(
            classify_obligation(&mixed_edge),
            ObligationOutcome::Resolved,
            "mixed Manual+default edge must be Resolved (has ≥1 default-firing route)"
        );

        // SkipOnMissingLicense alone fires-by-default → Resolved (not ConditionalResolved)
        let skip_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![skip_license_route()],
        );
        assert_eq!(
            classify_obligation(&skip_edge),
            ObligationOutcome::Resolved,
            "SkipOnMissingLicense fires by default → Resolved"
        );

        // Empty Multicast + Partial → HonestEmpty (unchanged)
        let empty_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![],
        );
        assert_eq!(
            classify_obligation(&empty_edge),
            ObligationOutcome::HonestEmpty,
            "empty fan-out must still be HonestEmpty"
        );
    }

    #[test]
    fn reachability_accessors_split_manual_from_default() {
        // Edge with one Manual route + one fires-by-default route.
        let edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![manual_route(), src_route()],
        );

        // default_reachable_routes: EXCLUDES the Manual route
        let default_r: Vec<_> = edge.default_reachable_routes().collect();
        assert_eq!(
            default_r.len(),
            1,
            "default_reachable_routes must exclude ManualBinding"
        );
        assert!(
            !default_r[0].conditions.contains(&Condition::ManualBinding),
            "the remaining route must not have ManualBinding"
        );

        // may_reachable_routes: INCLUDES both
        let may_r: Vec<_> = edge.may_reachable_routes().collect();
        assert_eq!(
            may_r.len(),
            2,
            "may_reachable_routes must include ManualBinding routes"
        );

        // all_routes: also includes both (resolution context)
        let all_r: Vec<_> = edge.all_routes().collect();
        assert_eq!(all_r.len(), 2, "all_routes must include all routes");
    }

    // ---- Task 3: UnknownReason payload + stratified breakdown ----

    fn unknown_route_with(reason: UnknownReason) -> Route {
        Route {
            target: RouteTarget::Unresolved,
            evidence: Evidence::Unknown(reason),
            conditions: vec![],
            witness: Witness::None,
            receiver_tier: None,
        }
    }

    /// (i) code-level invariant: every `ObligationOutcome::Unknown` edge's
    /// `Evidence::Unknown` route carries a reason — trivial by construction
    /// (the payload is required at construction, no zero-arg constructor
    /// survives), but pinned explicitly so a future regression that somehow
    /// reintroduces an un-tagged `Unknown` route fails a test, not just a
    /// silent diagnostic gap. Also pins `Evidence::kind()`'s projection.
    #[test]
    fn unknown_route_requires_reason_and_kind_projects_to_unknown() {
        let e = edge_with(
            EdgeKind::Call,
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![unknown_route_with(UnknownReason::MemberNotFound)],
        );
        assert_eq!(classify_obligation(&e), ObligationOutcome::Unknown);
        let route = &e.routes[0];
        assert_eq!(route.evidence.kind(), EvidenceKind::Unknown);
        match route.evidence {
            Evidence::Unknown(reason) => {
                assert_eq!(reason, UnknownReason::MemberNotFound);
            }
            _ => panic!("expected Evidence::Unknown(_), got {:?}", route.evidence),
        }
    }

    /// `Evidence::kind()` must project every `Unknown(_)` payload to the SAME
    /// `EvidenceKind::Unknown` — the boundary the committed semantic goldens
    /// rely on staying byte-identical (Task 3).
    #[test]
    fn evidence_kind_projection_ignores_reason_payload() {
        let a = Evidence::Unknown(UnknownReason::MemberNotFound);
        let b = Evidence::Unknown(UnknownReason::OverloadAmbiguous);
        assert_ne!(
            a, b,
            "distinct reasons must remain distinct Evidence values"
        );
        assert_eq!(
            a.kind(),
            b.kind(),
            "kind() must project away the reason payload"
        );
        assert_eq!(a.kind(), EvidenceKind::Unknown);
    }

    /// (ii) sum invariant: `unknown_reason_breakdown` is EXHAUSTIVE — every
    /// `Unknown`-classified edge contributes exactly one count, so the sum of
    /// the per-reason breakdown equals the total `Unknown` obligation count
    /// (mirrors `Histogram::of_edges().unknown`). Spans >=4 distinct reasons.
    #[test]
    fn unknown_reason_breakdown_sums_to_unknown_count() {
        let reasons = [
            UnknownReason::MemberNotFound,
            UnknownReason::OverloadAmbiguous,
            UnknownReason::CatalogMiss,
            UnknownReason::UntrackedReceiver,
            UnknownReason::CatalogMiss, // duplicate reason: must accumulate, not overwrite
        ];
        let mut edges: Vec<Edge> = reasons
            .iter()
            .map(|r| {
                edge_with(
                    EdgeKind::Call,
                    DispatchShape::Exact,
                    SetCompleteness::Complete,
                    vec![unknown_route_with(*r)],
                )
            })
            .collect();
        // Plus a non-Unknown edge, which must NOT contribute to the breakdown.
        edges.push(edge_with(
            EdgeKind::Call,
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![src_route()],
        ));

        let hist = Histogram::of_edges(&edges);
        assert_eq!(hist.unknown, 5, "sanity: 5 Unknown edges in the fixture");

        let breakdown = unknown_reason_breakdown(&edges);
        assert!(
            breakdown.len() >= 4,
            "fixture must span >=4 distinct reasons, got {}: {breakdown:?}",
            breakdown.len()
        );
        assert_eq!(
            breakdown.get(&UnknownReason::CatalogMiss).copied(),
            Some(2),
            "duplicate reasons must accumulate: {breakdown:?}"
        );
        let sum: usize = breakdown.values().sum();
        assert_eq!(
            sum, hist.unknown,
            "sum(unknownByReason) must equal the Unknown obligation count"
        );
    }

    // ---- Reason-split Task 2: new UnknownReason variants + receiver_tier ----

    fn unknown_route_with_tier(reason: UnknownReason, tier: TrustTier) -> Route {
        Route {
            receiver_tier: Some(tier),
            ..unknown_route_with(reason)
        }
    }

    /// Every new reason-split Task 2 variant renders a stable, distinct
    /// camelCase `as_str()` key (pinned so an accidental `Debug`-style rename
    /// or a duplicate key across variants fails a test, not a diagnostic
    /// consumer at runtime).
    #[test]
    fn reason_split_task2_variants_render_distinct_camel_case_keys() {
        assert_eq!(UnknownReason::ArityMismatch.as_str(), "arityMismatch");
        assert_eq!(
            UnknownReason::AbiCollapsedOverload.as_str(),
            "abiCollapsedOverload"
        );
        assert_eq!(
            UnknownReason::AccessFilteredOverload.as_str(),
            "accessFilteredOverload"
        );
        assert_eq!(UnknownReason::ObjectNotInGraph.as_str(), "objectNotInGraph");
        // Unchanged siblings still render their pre-existing keys.
        assert_eq!(
            UnknownReason::OverloadAmbiguous.as_str(),
            "overloadAmbiguous"
        );
        assert_eq!(UnknownReason::MemberNotFound.as_str(), "memberNotFound");

        let keys = [
            UnknownReason::ArityMismatch.as_str(),
            UnknownReason::AbiCollapsedOverload.as_str(),
            UnknownReason::AccessFilteredOverload.as_str(),
            UnknownReason::ObjectNotInGraph.as_str(),
            UnknownReason::OverloadAmbiguous.as_str(),
            UnknownReason::MemberNotFound.as_str(),
        ];
        let unique: std::collections::HashSet<&str> = keys.iter().copied().collect();
        assert_eq!(unique.len(), keys.len(), "every key must be distinct");
    }

    /// `unknown_receiver_tier_breakdown` (Task 2's ADDITIVE diagnostic):
    /// stratifies by `(reason, receiver_tier)`, accumulates duplicates, and
    /// its sum matches `unknown_reason_breakdown`'s sum exactly (same
    /// per-edge counting rule — see both functions' docs). `receiver_tier`
    /// is `None` for every non-`MemberNotFound` reason, `Some(tier)` only
    /// where explicitly tagged.
    #[test]
    fn unknown_receiver_tier_breakdown_sums_match_and_stratify_by_tier() {
        let edges = vec![
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![unknown_route_with_tier(
                    UnknownReason::MemberNotFound,
                    TrustTier::Workspace,
                )],
            ),
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![unknown_route_with_tier(
                    UnknownReason::MemberNotFound,
                    TrustTier::SymbolOnly,
                )],
            ),
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                // Duplicate (reason, tier) pair — must accumulate, not overwrite.
                vec![unknown_route_with_tier(
                    UnknownReason::MemberNotFound,
                    TrustTier::Workspace,
                )],
            ),
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                // ObjectNotInGraph never carries a tier.
                vec![unknown_route_with(UnknownReason::ObjectNotInGraph)],
            ),
        ];

        let hist = Histogram::of_edges(&edges);
        assert_eq!(hist.unknown, 4, "sanity: 4 Unknown edges in the fixture");

        let reason_breakdown = unknown_reason_breakdown(&edges);
        let tier_breakdown = unknown_receiver_tier_breakdown(&edges);

        let reason_sum: usize = reason_breakdown.values().sum();
        let tier_sum: usize = tier_breakdown.values().sum();
        assert_eq!(reason_sum, hist.unknown);
        assert_eq!(
            tier_sum, reason_sum,
            "unknown_receiver_tier_breakdown must count the same edges as \
             unknown_reason_breakdown — additive, never a different population"
        );

        assert_eq!(
            tier_breakdown.get(&(UnknownReason::MemberNotFound, Some(TrustTier::Workspace))),
            Some(&2),
            "duplicate (reason, tier) pairs must accumulate: {tier_breakdown:?}"
        );
        assert_eq!(
            tier_breakdown.get(&(UnknownReason::MemberNotFound, Some(TrustTier::SymbolOnly))),
            Some(&1)
        );
        assert_eq!(
            tier_breakdown.get(&(UnknownReason::ObjectNotInGraph, None)),
            Some(&1),
            "ObjectNotInGraph must report under a `None` tier key"
        );
    }
}
