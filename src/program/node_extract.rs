//! Extract object + routine nodes from one parsed `AlFile`.

use al_syntax::ir::{AlFile, ObjectKind, Param, RoutineKind};

use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::resolve::edge::{AbiEventKind, AbiRoutineKind};
use crate::program::resolve::event::{
    ParsedSubscriberArgs, PublisherKind, is_event_publisher, parse_event_subscriber_ir,
    publisher_include_sender, read_event_subscriber_instance,
};
use crate::program::resolve::receiver::unquote_identifier;
use crate::snapshot::TrustTier;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Public,
    Local,
    Internal,
    Protected,
}

impl Access {
    fn from_modifier(m: Option<&str>) -> Access {
        match m.map(str::to_ascii_lowercase).as_deref() {
            Some("local") => Access::Local,
            Some("internal") => Access::Internal,
            Some("protected") => Access::Protected,
            _ => Access::Public,
        }
    }
}

/// A losslessly-typed reference to another AL object as written in an object
/// property (`SourceTable`, `TableNo`) or a page-control target: either a
/// numeric AL object id or a name. Kept distinct from a plain `String` so a
/// numeric reference (`SourceTable = 36`) is never confused with a
/// digit-only name, and so [`ResolveIndex::resolve_object_ref`] can dispatch
/// each shape to the correct index without re-parsing.
///
/// [`ResolveIndex::resolve_object_ref`]: crate::program::resolve::index::ResolveIndex::resolve_object_ref
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectRef {
    /// A name reference. `raw` preserves the as-written (unquoted) text for
    /// display; `normalized_lc` is the lowercased form used for matching.
    Name { raw: String, normalized_lc: String },
    /// A numeric AL object id reference.
    Id(i64),
}

/// The kind of one Page/PageExtension layout control, from its raw grammar
/// section keyword (`part` / `systempart` / `usercontrol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageControlKind {
    Part,
    SystemPart,
    UserControl,
}

/// One `part` / `systempart` / `usercontrol` layout control on a
/// Page/PageExtension, in document order. Consumed by Task 7's Step 0 in
/// `infer_receiver_type` to resolve `CurrPage.<part>.Page` subpage-instance
/// receivers (beyond-1B.3b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageControlNode {
    pub name_lc: String,
    pub kind: PageControlKind,
    pub target: ObjectRef,
}

/// One report `dataitem(Name; "Source Table")` declaration — Report /
/// ReportExtension only, document order (dataitem-receivers plan, Task 1).
/// Mirrors [`PageControlNode`]: `name_lc` is the lowercased UNQUOTED dataitem
/// name (`al_syntax::ir::ObjectDecl.report_dataitems` is already
/// outer-quote-stripped, `ident_text`); `source_table` is the RAW `ObjectRef`
/// parsed exactly like `SourceTable`/`TableNo` — resolved lazily via
/// [`crate::program::resolve::receiver::resolve_source_table_ref`] at the
/// same fail-closed call sites Page/PageExtension/Codeunit already use, never
/// pre-resolved here (keeps `ObjectNode` topology-independent, matching every
/// other `*Ref` field on this struct).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataitemNode {
    pub name_lc: String,
    pub name: String,
    pub source_table: ObjectRef,
}

/// One table field surface entry (Table / TableExtension only) — Task 3
/// (record-field chains). `name_lc` is the lowercased, UNQUOTED field name
/// (mirrors [`RoutineNode`]'s `name_lc`/`RoutineNodeId::name_lc` convention —
/// both a quoted (`"Error Message"`) and unquoted (`BlobField`) AL member
/// reference normalize to the SAME lowercased text at the consumption site,
/// see `receiver::infer_compound_member_receiver`'s `member_lc`). `type_text`
/// is the RAW declared type text, verbatim (`"Blob"`, `"Enum \"Doc
/// Status\""`, `"Integer"`, …) — deliberately UNCLASSIFIED here: the consumer
/// (`ResolveIndex::field_in_table` → `receiver::classify_type_text`) is the
/// single place that turns text into a [`crate::program::resolve::receiver::ParsedType`],
/// so a field's type is classified via the SAME strict logic every other
/// declared type (param/local/return) goes through — never a separate,
/// possibly-diverging path (e.g. `FieldDecl::is_blob_like`, which also flags
/// Media/MediaSet and would falsely broaden a Media field into the Blob
/// catalog if used for classification instead of the declared text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldNode {
    pub name_lc: String,
    pub type_text: String,
}

#[derive(Debug, Clone)]
pub struct ObjectNode {
    pub id: ObjectNodeId,
    pub name: String,
    pub declared_id: Option<i64>,
    pub extends_target: Option<String>,
    pub implements: Vec<String>,
    pub tier: TrustTier,
    /// The `SourceTable` object property — Page/PageExtension/Report/
    /// ReportExtension only; `None` for every other kind (and when the
    /// property is absent). Seeds implicit-`Rec` table resolution (Tasks 5–7).
    pub source_table: Option<ObjectRef>,
    /// The `TableNo` object property — Codeunit only; `None` otherwise.
    pub table_no: Option<ObjectRef>,
    /// `true` when the `SourceTable` property carried a trailing `temporary`
    /// marker (`SourceTable = X, Temporary` / `SourceTable = X temporary`).
    /// Always `false` when `source_table` is `None`.
    pub source_table_temporary: bool,
    /// Page/PageExtension layout controls (`part`/`systempart`/`usercontrol`),
    /// document order. Empty for every other object kind.
    pub page_controls: Vec<PageControlNode>,
    /// Table fields (Table / TableExtension only), document order — Task 3.
    /// Populated from `FieldDecl` (source, `extract_nodes`) or `AbiField`
    /// (ABI, `abi_ingest::ingest_abi` — Task 2's Subtype-qualified
    /// `parse_field`). Empty for every other object kind. Consumed by
    /// `ResolveIndex::field_in_table` for the `Rec."Field".X()` record-field
    /// chain arm in `receiver::infer_compound_member_receiver`.
    pub fields: Vec<FieldNode>,
    /// Report `dataitem(Name; "Source Table")` declarations (Report /
    /// ReportExtension only), document order — dataitem-receivers plan
    /// (Task 1). Empty for every other object kind. Consumed by
    /// `receiver::resolve_dataitem_source_table` (Step 2b's dataitem-NAME
    /// receiver lookup, and the report implicit-Rec fallback).
    pub dataitems: Vec<DataitemNode>,
}

#[derive(Debug, Clone)]
pub struct RoutineNode {
    pub id: RoutineNodeId,
    pub name: String,
    pub is_trigger: bool,
    pub access: Access,
    pub tier: TrustTier,
    /// All `[EventSubscriber]` attributes parsed from this routine, in source order.
    pub event_subscribers: Vec<ParsedSubscriberArgs>,
    /// True when the owning object has `EventSubscriberInstance = Manual`.
    pub subscriber_instance_manual: bool,
    /// The event-publisher kind when this routine carries an `[IntegrationEvent]`,
    /// `[BusinessEvent]`, or `[InternalEvent]` attribute; `None` otherwise.
    pub publisher_kind: Option<PublisherKind>,
    /// The publisher attribute's `IncludeSender` flag (Task 1) — tri-state:
    /// `Some(true)`/`Some(false)` when the attribute's first arg parsed to a
    /// literal boolean; `None` when this routine is not a publisher at all, or
    /// the arg could not be read (fail-closed unknown). Populated at
    /// ingestion: source routines via
    /// `crate::program::resolve::event::publisher_include_sender`; ABI
    /// routines via `abi_ingest::abi_publisher_include_sender`; a
    /// platform-synthetic publisher (`build::inject_platform_event_publishers`)
    /// always carries `None` (it has no real `[IntegrationEvent]` attribute to
    /// read, and platform DB-trigger/lifecycle events never legally prepend a
    /// Sender). SINGLE SOURCE OF TRUTH consumed via
    /// `crate::program::resolve::event::subscriber_arity_bound` by BOTH the
    /// `index.rs` subscriber-wiring candidate filter and
    /// `differential::verify_event_subscriber_route`'s independent re-check —
    /// see that function's doc for why the `+1` Sender tolerance must be
    /// CONDITIONAL on this field, never blanket.
    pub include_sender: Option<bool>,
    /// ABI-only: the routine kind for ABI-boundary routing. `None` for source routines.
    pub abi_routine_kind: Option<AbiRoutineKind>,
    /// ABI-only: the event kind for ABI-boundary publisher annotation. `None` for source routines.
    pub abi_event_kind: Option<AbiEventKind>,
    /// Content key distinguishing SOURCE routines that collide onto the same
    /// `RoutineNodeId` (source `sig_fp` is always `0` — see node.rs): the
    /// lowercased, `|`-joined parameter-type-text sequence, computed by
    /// [`param_sig_key`]. Two re-parses of the SAME declaration always share
    /// this key; two genuine same-name/same-arity overloads (differing only
    /// by parameter TYPE) always differ in it. Used by
    /// `build::dedup_routines_preserving_genuine_overloads` (beyond-1B.3b
    /// Task 2 review fix) to collapse a duplicate-id run to its true
    /// canonical count regardless of how many times the owning object itself
    /// was duplicated. Always `String::new()` for ABI/SymbolOnly routines —
    /// those already carry a non-zero `sig_fp` in their `RoutineNodeId` when
    /// signatures differ, so same-id runs there are already true duplicates.
    pub param_sig_key: String,
    /// Declared return-type text, verbatim (e.g. `"Codeunit X"`) for a SOURCE
    /// routine (copied from `RoutineDecl.return_type`), or the reconstructed
    /// SOURCE-SHAPED text for an ABI/SymbolOnly routine (Task 2 — see
    /// `abi_ingest::ingest_abi` and
    /// `crate::engine::deps::symbol_reference::reconstruct_return_type_text`).
    /// `None` for a procedure/trigger with no return type, or when an ABI
    /// return type could not be safely reconstructed (fail-closed — see that
    /// function's doc). Not yet consumed by any resolver; additive plumbing
    /// for a future compound-receiver Phase-A step (`Func().Method()`).
    pub return_type: Option<String>,
    /// The structured `(name, id)` cross-validation pair from an ABI return
    /// type's `Subtype` (Task 2), present only when the underlying
    /// `AbiRoutine::return_type_id` carried both fields. Always `None` for a
    /// SOURCE routine (no equivalent raw JSON identity to carry). Reachable
    /// via the SAME `RoutineNodeId` lookup (`graph.routines.binary_search_by`)
    /// regardless of which `RouteTarget` shape a consumer resolves through —
    /// see `AbiRoutine::return_type_id`'s doc for the full cross-validation
    /// rationale (Task 3 consumes this; Task 2 only carries it).
    pub return_type_id: Option<(String, i64)>,
    /// `true` when this node is the arbitrary SURVIVOR of a dedup collapse
    /// that folded ≥2 raw ABI overload entries onto the same `RoutineNodeId`
    /// (Task 3 review fix). An ABI routine's [`param_sig_key`] is always
    /// `String::new()` (see that field's doc) — `AbiParameter::type_text`
    /// carries only a parameter's OUTER type keyword, never its `Subtype`,
    /// so two genuinely distinct same-name/same-arity overloads differing
    /// only by an object-typed parameter's Subtype (`Get(X: Codeunit A)` vs
    /// `Get(X: Codeunit B)`) hash-collide onto the identical `RoutineNodeId`
    /// and `build::dedup_routines_preserving_genuine_overloads` silently
    /// keeps only the first raw entry. That survivor's `return_type` /
    /// `return_type_id` are therefore UNTRUSTWORTHY BY CONSTRUCTION — they
    /// belong to only ONE of the ≥2 real declarations, chosen arbitrarily by
    /// raw JSON order, and a downstream consumer has no way to tell which.
    /// Set `true` ONLY when ≥2 raw ABI (`TrustTier::SymbolOnly`) entries
    /// shared the node id; always `false` for a SOURCE routine (whose
    /// `param_sig_key` is real parsed param-type content, so a genuine
    /// same-id collapse there is always a true re-parse duplicate of the
    /// SAME declaration — content-identical, safe to trust). Consumed by
    /// `resolver::routine_node_for_type_query` /
    /// `resolver::resolve_abi_prefix_routine` and
    /// `receiver::receiver_from_routine_node` (Task 3's cross-object
    /// call-result chain typing, `Var.Method().X()`) — both DECLINE
    /// (`Unknown(CompoundReceiver)`) rather than read a collapsed survivor's
    /// return type, fail-closed.
    pub abi_overload_collapsed: bool,
}

/// Lowercased, `|`-joined parameter TYPE-TEXT sequence for a SOURCE routine's
/// params — the content key [`RoutineNode::param_sig_key`] stores. Mirrors
/// the normalization in `abi_ingest::param_type_fp` (lowercase + `|`-join),
/// computed here from source `Param.ty` rather than ABI `AbiParameter::type_text`.
/// An absent/unparsed type normalizes to `""`. Two params that BOTH fail to
/// parse a type are therefore indistinguishable by this key alone, which
/// could over-collapse a genuine overload pair in that narrow pathological
/// corner (same failure mode the pre-Task-2 blanket `dedup_by` had for every
/// routine); ordinary parsed source does not hit this, since `Param.ty` is
/// populated whenever the parameter list itself parsed.
fn param_sig_key(params: &[Param]) -> String {
    params
        .iter()
        .map(|p| p.ty.as_deref().unwrap_or("").trim().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("|")
}

/// Parse an object-property value (`SourceTable`/`TableNo`) or a page-control
/// target into an [`ObjectRef`], plus whether a trailing `temporary` marker
/// was present and stripped. A numeric value → [`ObjectRef::Id`]; anything
/// else → [`ObjectRef::Name`] with quotes stripped (mirrors the unquoting
/// [`crate::program::resolve::receiver::classify_type_text`] applies to a
/// `Record <name>` type).
fn parse_object_ref_value(value: &str) -> (ObjectRef, bool) {
    let (base, is_temporary) = strip_temporary_marker(value.trim());
    let base = base.trim();
    if let Ok(n) = base.parse::<i64>() {
        (ObjectRef::Id(n), is_temporary)
    } else {
        let raw = unquote_identifier(base);
        let normalized_lc = raw.to_ascii_lowercase();
        (ObjectRef::Name { raw, normalized_lc }, is_temporary)
    }
}

/// Strip a trailing `temporary` marker (case-insensitive) from an
/// object-property value's name/id portion, separated from it by whitespace
/// (`SourceTable = Customer temporary`, mirroring
/// [`crate::program::resolve::receiver::classify_type_text`]'s `Record <name>
/// temporary` handling) or by a comma (`SourceTable = Customer, Temporary`).
/// Returns the remaining text and whether a marker was found.
///
/// Stripping requires an explicit separator immediately before the keyword,
/// so a bare identifier that merely ENDS in "temporary" (e.g. a table
/// literally named `MyTemporary`) is left untouched.
fn strip_temporary_marker(s: &str) -> (&str, bool) {
    let trimmed_end = s.trim_end();
    let lower = trimmed_end.to_ascii_lowercase();
    let Some(prefix_len) = lower.strip_suffix("temporary").map(str::len) else {
        return (trimmed_end, false);
    };
    let prefix = &trimmed_end[..prefix_len];
    let has_separator =
        matches!(prefix.chars().next_back(), Some(c) if c.is_whitespace() || c == ',');
    if !has_separator {
        return (trimmed_end, false);
    }
    let remaining = prefix.trim_end();
    let remaining = remaining
        .strip_suffix(',')
        .map(str::trim_end)
        .unwrap_or(remaining);
    (remaining, true)
}

/// Map a raw page-control kind string (`"part"` / `"systempart"` /
/// `"usercontrol"` — the only values the lowerer emits) to [`PageControlKind`].
/// Returns `None` for anything else (defensive — never expected in practice).
fn page_control_kind(raw: &str) -> Option<PageControlKind> {
    match raw {
        "part" => Some(PageControlKind::Part),
        "systempart" => Some(PageControlKind::SystemPart),
        "usercontrol" => Some(PageControlKind::UserControl),
        _ => None,
    }
}

pub fn extract_nodes(
    app: AppRef,
    file: &AlFile,
    tier: TrustTier,
    objects: &mut Vec<ObjectNode>,
    routines: &mut Vec<RoutineNode>,
) {
    for obj in &file.objects {
        let key = match obj.id {
            Some(n) => ObjKey::Id(n),
            None => ObjKey::Name(obj.name.to_ascii_lowercase()),
        };
        let obj_id = ObjectNodeId {
            app,
            kind: obj.kind,
            key,
        };

        // SourceTable — Page/PageExtension/Report/ReportExtension only.
        let mut source_table = None;
        let mut source_table_temporary = false;
        if matches!(
            obj.kind,
            ObjectKind::Page
                | ObjectKind::PageExtension
                | ObjectKind::Report
                | ObjectKind::ReportExtension
        ) && let Some(prop) = obj.properties.iter().find(|p| p.name == "sourcetable")
        {
            let (r, is_temp) = parse_object_ref_value(&prop.value);
            source_table = Some(r);
            source_table_temporary = is_temp;
        }

        // TableNo — Codeunit only.
        let table_no = if obj.kind == ObjectKind::Codeunit {
            obj.properties
                .iter()
                .find(|p| p.name == "tableno")
                .map(|p| parse_object_ref_value(&p.value).0)
        } else {
            None
        };

        // Page controls — Page/PageExtension only, document order.
        let page_controls = if matches!(obj.kind, ObjectKind::Page | ObjectKind::PageExtension) {
            obj.page_controls
                .iter()
                .filter_map(|pc| {
                    Some(PageControlNode {
                        name_lc: pc.name.to_ascii_lowercase(),
                        kind: page_control_kind(&pc.kind)?,
                        target: parse_object_ref_value(&pc.target).0,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        // Table fields — Table/TableExtension only, document order (Task 3).
        // `f.name` is already unquoted (`lower_field`'s `ident_text`, mirrors
        // `RoutineDecl.name`), so only lowercasing is needed here — the same
        // convention `RoutineNode::id.name_lc` uses for routine names.
        let fields = if matches!(obj.kind, ObjectKind::Table | ObjectKind::TableExtension) {
            obj.fields
                .iter()
                .map(|f| FieldNode {
                    name_lc: f.name.to_ascii_lowercase(),
                    type_text: f.data_type.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };

        // Report dataitems — Report/ReportExtension only, document order (Task 1,
        // dataitem-receivers plan). `d.0`/`d.1` are already outer-quote-stripped
        // (`ident_text`, `al_syntax::lower::collect_report_dataitems`); the shared
        // `parse_object_ref_value` still normalizes the table half losslessly
        // (numeric vs quoted-name), mirroring `SourceTable`/`TableNo` above.
        let dataitems = if matches!(obj.kind, ObjectKind::Report | ObjectKind::ReportExtension) {
            obj.report_dataitems
                .iter()
                .map(|(name, table)| DataitemNode {
                    name_lc: name.to_ascii_lowercase(),
                    name: name.clone(),
                    source_table: parse_object_ref_value(table).0,
                })
                .collect()
        } else {
            Vec::new()
        };

        objects.push(ObjectNode {
            id: obj_id.clone(),
            name: obj.name.clone(),
            declared_id: obj.id,
            extends_target: obj.extends_target.clone(),
            implements: obj.implements.clone(),
            tier,
            source_table,
            table_no,
            source_table_temporary,
            page_controls,
            fields,
            dataitems,
        });
        // Computed once per object — same value for every routine in the object.
        let subscriber_instance_manual = read_event_subscriber_instance(obj);
        for r in &obj.routines {
            let has_sub_attr = r.attributes.iter().any(|a| a == "eventsubscriber");
            let event_subscribers: Vec<ParsedSubscriberArgs> = if has_sub_attr {
                r.attributes_parsed
                    .iter()
                    .filter(|a| a.name.eq_ignore_ascii_case("eventsubscriber"))
                    .filter_map(|a| parse_event_subscriber_ir(a, &file.ir))
                    .collect()
            } else {
                vec![]
            };
            let publisher_kind = is_event_publisher(r);
            // Only meaningful when `publisher_kind.is_some()`; the parser itself
            // already filters to a publisher attribute (integrationevent /
            // businessevent / internalevent), so this is always `None` on a
            // non-publisher routine.
            let include_sender = publisher_include_sender(r, &file.ir);
            routines.push(RoutineNode {
                id: RoutineNodeId {
                    object: obj_id.clone(),
                    name_lc: r.name.to_ascii_lowercase(),
                    enclosing_member_lc: r
                        .enclosing_member
                        .as_ref()
                        .map(|(n, _)| n.to_ascii_lowercase()),
                    params_count: r.params.len(),
                    sig_fp: 0,
                },
                name: r.name.clone(),
                is_trigger: matches!(r.kind, RoutineKind::Trigger),
                access: Access::from_modifier(r.access_modifier.as_deref()),
                tier,
                event_subscribers,
                subscriber_instance_manual,
                publisher_kind,
                include_sender,
                abi_routine_kind: None,
                abi_event_kind: None,
                param_sig_key: param_sig_key(&r.params),
                return_type: r.return_type.clone(),
                return_type_id: None,
                abi_overload_collapsed: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::{AppRef, ObjKey};
    use crate::snapshot::TrustTier;

    #[test]
    fn extracts_object_and_routines_with_access() {
        let src = r#"
codeunit 50100 "Sales Helper"
{
    procedure Post() begin end;
    local procedure Helper() begin end;
}
"#;
        let file = al_syntax::parse(src);
        let mut objs = Vec::new();
        let mut routs = Vec::new();
        extract_nodes(
            AppRef(0),
            &file,
            TrustTier::Workspace,
            &mut objs,
            &mut routs,
        );
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].id.key, ObjKey::Id(50100));
        assert_eq!(objs[0].name, "Sales Helper");
        assert_eq!(routs.len(), 2);
        let post = routs.iter().find(|r| r.id.name_lc == "post").unwrap();
        assert_eq!(post.access, Access::Public);
        let helper = routs.iter().find(|r| r.id.name_lc == "helper").unwrap();
        assert_eq!(helper.access, Access::Local);
        assert!(!post.is_trigger);
    }

    /// Task 2 invariant (b): a source routine `procedure P(): Codeunit X` has
    /// `return_type == Some("Codeunit X")` on its extracted `RoutineNode`
    /// (copied verbatim from `RoutineDecl.return_type`); a routine with no
    /// return type declared stays `None`.
    #[test]
    fn extracts_source_routine_return_type() {
        let src = r#"
codeunit 50100 "Sales Helper"
{
    procedure GetHelper(): Codeunit X begin end;
    procedure Post() begin end;
}
"#;
        let file = al_syntax::parse(src);
        let mut objs = Vec::new();
        let mut routs = Vec::new();
        extract_nodes(
            AppRef(0),
            &file,
            TrustTier::Workspace,
            &mut objs,
            &mut routs,
        );
        let get_helper = routs.iter().find(|r| r.id.name_lc == "gethelper").unwrap();
        assert_eq!(get_helper.return_type.as_deref(), Some("Codeunit X"));
        let post = routs.iter().find(|r| r.id.name_lc == "post").unwrap();
        assert_eq!(post.return_type, None);
    }

    /// Parse `src` and return every extracted `ObjectNode`, document order.
    fn extract_objs(src: &str) -> Vec<ObjectNode> {
        let file = al_syntax::parse(src);
        let mut objs = Vec::new();
        let mut routs = Vec::new();
        extract_nodes(
            AppRef(0),
            &file,
            TrustTier::Workspace,
            &mut objs,
            &mut routs,
        );
        objs
    }

    // -- source_table / page_controls (Page) ----------------------------------

    #[test]
    fn page_source_table_name_and_part_control() {
        let src = r#"
page 50100 "Card"
{
    SourceTable = Customer;
    layout { area(Content) { part(Lines; "Sales Line Subform") { } } }
}
"#;
        let objs = extract_objs(src);
        assert_eq!(objs.len(), 1);
        let page = &objs[0];
        assert_eq!(
            page.source_table,
            Some(ObjectRef::Name {
                raw: "Customer".to_string(),
                normalized_lc: "customer".to_string(),
            })
        );
        assert!(!page.source_table_temporary);
        assert_eq!(page.table_no, None, "TableNo is Codeunit-only");
        assert_eq!(page.page_controls.len(), 1);
        assert_eq!(
            page.page_controls[0],
            PageControlNode {
                name_lc: "lines".to_string(),
                kind: PageControlKind::Part,
                target: ObjectRef::Name {
                    raw: "Sales Line Subform".to_string(),
                    normalized_lc: "sales line subform".to_string(),
                },
            }
        );
    }

    #[test]
    fn page_source_table_numeric_id() {
        let src = r#"
page 50101 "NumCard"
{
    SourceTable = 36;
    layout { area(Content) { } }
}
"#;
        let objs = extract_objs(src);
        assert_eq!(objs[0].source_table, Some(ObjectRef::Id(36)));
        assert!(!objs[0].source_table_temporary);
    }

    #[test]
    fn source_table_trailing_temporary_marker_stripped() {
        let src = r#"
page 50102 "TempCard"
{
    SourceTable = Customer, Temporary;
    layout { area(Content) { } }
}
"#;
        let objs = extract_objs(src);
        assert_eq!(
            objs[0].source_table,
            Some(ObjectRef::Name {
                raw: "Customer".to_string(),
                normalized_lc: "customer".to_string(),
            }),
            "the temporary marker must not leak into the resolved name"
        );
        assert!(objs[0].source_table_temporary);
    }

    #[test]
    fn page_controls_preserve_document_order() {
        let src = r#"
page 50103 "MultiControl"
{
    SourceTable = Customer;
    layout
    {
        area(Content)
        {
            part(First; "Part A") { }
            systempart(Second; Notes) { }
            usercontrol(Third; "MyAddIn") { }
        }
    }
}
"#;
        let objs = extract_objs(src);
        let controls = &objs[0].page_controls;
        assert_eq!(controls.len(), 3);
        assert_eq!(controls[0].name_lc, "first");
        assert_eq!(controls[0].kind, PageControlKind::Part);
        assert_eq!(controls[1].name_lc, "second");
        assert_eq!(controls[1].kind, PageControlKind::SystemPart);
        assert_eq!(controls[2].name_lc, "third");
        assert_eq!(controls[2].kind, PageControlKind::UserControl);
    }

    // -- table_no (Codeunit) ---------------------------------------------------

    #[test]
    fn codeunit_table_no_name() {
        let src = r#"
codeunit 50104 "Item Helper"
{
    TableNo = Item;
}
"#;
        let objs = extract_objs(src);
        assert_eq!(
            objs[0].table_no,
            Some(ObjectRef::Name {
                raw: "Item".to_string(),
                normalized_lc: "item".to_string(),
            })
        );
        assert_eq!(objs[0].source_table, None, "SourceTable is not Codeunit");
        assert!(objs[0].page_controls.is_empty());
    }

    // -- Table: no node-fidelity fields -----------------------------------------

    #[test]
    fn table_object_has_no_node_fidelity_fields() {
        let src = r#"
table 50105 "Plain Table"
{
    fields { field(1; "No."; Code[20]) { } }
}
"#;
        let objs = extract_objs(src);
        assert_eq!(objs[0].source_table, None);
        assert_eq!(objs[0].table_no, None);
        assert!(!objs[0].source_table_temporary);
        assert!(objs[0].page_controls.is_empty());
    }
}
