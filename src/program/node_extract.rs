//! Extract object + routine nodes from one parsed `AlFile`.

use al_syntax::ir::{AlFile, ObjectKind, Param, RoutineKind};

use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::resolve::edge::{AbiEventKind, AbiRoutineKind};
use crate::program::resolve::event::{
    ParsedSubscriberArgs, PublisherKind, is_event_publisher, parse_event_subscriber_ir,
    read_event_subscriber_instance,
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
                abi_routine_kind: None,
                abi_event_kind: None,
                param_sig_key: param_sig_key(&r.params),
                return_type: r.return_type.clone(),
                return_type_id: None,
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
