//! Event attribute parsing primitives — Plan 1B.2 Phase 4b Task 1.
//!
//! Reads `[EventSubscriber(...)]` attribute arguments from the IR, detects
//! publisher routines by attribute name, and reads the
//! `EventSubscriberInstance = Manual` codeunit property.
//!
//! Clean-room: no L3 `event_graph` imports.

use al_syntax::ir::{AttributeIr, ExprId, ExprKind, Ir, Literal, ObjectDecl, RoutineDecl};

// ─────────────────────────────────────────────────────────────────────────────
// Subscriber argument parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Typed result of parsing an `[EventSubscriber(…)]` attribute's positional args.
///
/// All string fields are lowercased and unquoted.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSubscriberArgs {
    /// Publisher object type, lowercased (e.g. `"codeunit"`).
    pub publisher_object_type: String,
    /// Publisher object name, unquoted and lowercased.
    pub publisher_name: String,
    /// Event procedure name, unquoted and lowercased.
    pub event_name: String,
    /// Optional element filter — `None` when absent or when the arg is an empty
    /// string literal.
    pub element: Option<String>,
    pub skip_on_missing_license: bool,
    pub skip_on_missing_permission: bool,
}

/// Parse `[EventSubscriber(ObjectType::Codeunit, Codeunit::"Pub", 'OnAfterX',
/// 'Element', SkipLicense, SkipPermission)]` from the IR expression arena.
///
/// Arg mapping:
/// - 0 `QualifiedEnum.value`                      → `publisher_object_type` (lc)
/// - 1 `DatabaseReference` / `Member.member` / …  → `publisher_name` (unquoted, lc)
/// - 2 `Literal::Text`                            → `event_name` (stripped, lc)
/// - 3 `Literal::Text`                            → `element` (`None` if absent/empty)
/// - 4 `Literal::Bool`                            → `skip_on_missing_license` (absent → false)
/// - 5 `Literal::Bool`                            → `skip_on_missing_permission` (absent → false)
///
/// Returns `None` when arg 0/1/2 is missing or of an unrecognised kind.
pub fn parse_event_subscriber_ir(attr: &AttributeIr, ir: &Ir) -> Option<ParsedSubscriberArgs> {
    if attr.args.len() < 3 {
        return None;
    }

    // Arg 0: `ObjectType::Codeunit` → QualifiedEnum { value: "Codeunit" }
    let publisher_object_type = match &ir.expr(attr.args[0]).kind {
        ExprKind::QualifiedEnum { value, .. } => value.to_ascii_lowercase(),
        _ => return None,
    };

    // Arg 1: `Codeunit::"Pub"` — several IR shapes depending on grammar parse path.
    let publisher_name = resolve_publisher_name(ir, attr.args[1])?;

    // Arg 2: `'OnAfterX'` → Literal::Text (raw text, single-quoted in AL).
    let event_name = match &ir.expr(attr.args[2]).kind {
        ExprKind::Literal(Literal::Text(s)) => strip_al_string(s).to_ascii_lowercase(),
        _ => return None,
    };
    if event_name.is_empty() {
        return None;
    }

    // Arg 3 (optional): element filter — absent or empty string literal → None.
    let element = attr.args.get(3).and_then(|&id| match &ir.expr(id).kind {
        ExprKind::Literal(Literal::Text(s)) => {
            let v = strip_al_string(s).to_ascii_lowercase();
            if v.is_empty() { None } else { Some(v) }
        }
        _ => None,
    });

    // Arg 4 (optional): skip_on_missing_license; absent → false.
    let skip_on_missing_license = attr
        .args
        .get(4)
        .is_some_and(|&id| matches!(&ir.expr(id).kind, ExprKind::Literal(Literal::Bool(true))));

    // Arg 5 (optional): skip_on_missing_permission; absent → false.
    let skip_on_missing_permission = attr
        .args
        .get(5)
        .is_some_and(|&id| matches!(&ir.expr(id).kind, ExprKind::Literal(Literal::Bool(true))));

    Some(ParsedSubscriberArgs {
        publisher_object_type,
        publisher_name,
        event_name,
        element,
        skip_on_missing_license,
        skip_on_missing_permission,
    })
}

/// Resolve arg 1 of an `[EventSubscriber]` attribute to the publisher object
/// name (unquoted, lowercased).
///
/// Handles:
/// - `DatabaseReference("Codeunit::\"Pub\"")` — split on `::`, strip quotes on RHS
/// - `Member { member: "\"Pub\"", .. }`        — strip quotes from member text
/// - `QualifiedEnum { value, .. }`              — already unquoted by `ident_text`
/// - `Identifier` / `QuotedIdentifier`          — strip quotes, lowercase
fn resolve_publisher_name(ir: &Ir, id: ExprId) -> Option<String> {
    match &ir.expr(id).kind {
        ExprKind::DatabaseReference(t) => {
            let name_part = match t.split_once("::") {
                Some((_, n)) => n,
                None => t.as_str(),
            };
            Some(strip_al_string(name_part).to_ascii_lowercase())
        }
        ExprKind::Member { member, .. } => Some(strip_al_string(member).to_ascii_lowercase()),
        ExprKind::QualifiedEnum { value, .. } => Some(value.to_ascii_lowercase()),
        ExprKind::Identifier(s) | ExprKind::QuotedIdentifier(s) => {
            Some(strip_al_string(s).to_ascii_lowercase())
        }
        _ => None,
    }
}

/// Strip exactly ONE layer of surrounding single or double quotes from a raw AL
/// string / identifier token.  Returns the inner slice (already trimmed).
fn strip_al_string(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 {
        let b = s.as_bytes();
        let first = b[0];
        let last = b[s.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// Publisher kind detection
// ─────────────────────────────────────────────────────────────────────────────

/// The event-publisher kind encoded by a routine's attribute.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PublisherKind {
    Integration,
    Business,
    Internal,
    /// A platform-generated table event (`OnAfter*Event` / `OnBefore*Event` +
    /// field validate) with NO publisher routine in source. Carried by a
    /// SYNTHETIC publisher routine injected on the table so that subscribers to
    /// the platform's implicit DB-trigger / validate events (a large class of
    /// real integration wiring) resolve instead of orphaning. See
    /// [`is_platform_table_event`] and `build::inject_platform_event_publishers`.
    Platform,
}

/// True when `name_lc` is a platform-generated TABLE event that AL raises
/// implicitly on a DB operation (insert/modify/delete/rename) or a field
/// validate. These have NO publisher routine in source — a `[EventSubscriber(
/// ObjectType::Table, Database::X, 'OnAfterDeleteEvent', …)]` targeting one binds
/// to a synthetic [`PublisherKind::Platform`] publisher on the table.
pub fn is_platform_table_event(name_lc: &str) -> bool {
    matches!(
        name_lc,
        "onbeforeinsertevent"
            | "onafterinsertevent"
            | "onbeforemodifyevent"
            | "onaftermodifyevent"
            | "onbeforedeleteevent"
            | "onafterdeleteevent"
            | "onbeforerenameevent"
            | "onafterrenameevent"
            | "onbeforevalidateevent"
            | "onaftervalidateevent"
    )
}

/// Canonical PascalCase display name for a platform table event; `name_lc` must
/// satisfy [`is_platform_table_event`]. Falls back to a generic label otherwise.
pub fn platform_event_display_name(name_lc: &str) -> &'static str {
    match name_lc {
        "onbeforeinsertevent" => "OnBeforeInsertEvent",
        "onafterinsertevent" => "OnAfterInsertEvent",
        "onbeforemodifyevent" => "OnBeforeModifyEvent",
        "onaftermodifyevent" => "OnAfterModifyEvent",
        "onbeforedeleteevent" => "OnBeforeDeleteEvent",
        "onafterdeleteevent" => "OnAfterDeleteEvent",
        "onbeforerenameevent" => "OnBeforeRenameEvent",
        "onafterrenameevent" => "OnAfterRenameEvent",
        "onbeforevalidateevent" => "OnBeforeValidateEvent",
        "onaftervalidateevent" => "OnAfterValidateEvent",
        _ => "PlatformEvent",
    }
}

/// Classify a routine as an event publisher from its lowercased `attributes`
/// list.  Returns the first matching kind; `None` when the routine carries no
/// publisher attribute.
pub fn is_event_publisher(decl: &RoutineDecl) -> Option<PublisherKind> {
    for attr in &decl.attributes {
        match attr.as_str() {
            "integrationevent" => return Some(PublisherKind::Integration),
            "businessevent" => return Some(PublisherKind::Business),
            "internalevent" => return Some(PublisherKind::Internal),
            _ => {}
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// EventSubscriberInstance property
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` when the object's `EventSubscriberInstance` property is set
/// to `Manual` (case-insensitive).  Accepts both the bare form (`Manual`) and
/// the qualified enum form (`EventSubscriberInstance::Manual`).
pub fn read_event_subscriber_instance(obj: &ObjectDecl) -> bool {
    obj.properties.iter().any(|p| {
        if p.name != "eventsubscriberinstance" {
            return false;
        }
        // Strip an optional `Enum::` qualifier from the raw value text.
        let v = match p.value.rfind("::") {
            Some(i) => &p.value[i + 2..],
            None => p.value.as_str(),
        };
        v.trim().eq_ignore_ascii_case("manual")
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // All tests parse real AL source via `al_syntax::parse` to build a genuine IR
    // rather than constructing arena nodes by hand.  This validates the full
    // lowerer → IR → event-parser pipeline.

    // ── parse_event_subscriber_ir ─────────────────────────────────────────────

    #[test]
    fn full_six_args_empty_element_license_true() {
        let src = r#"codeunit 50100 Sub
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Pub", 'OnAfterX', '', true, false)]
    local procedure OnAfterX()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let attr = &af.objects[0].routines[0].attributes_parsed[0];
        assert_eq!(
            parse_event_subscriber_ir(attr, &af.ir),
            Some(ParsedSubscriberArgs {
                publisher_object_type: "codeunit".into(),
                publisher_name: "pub".into(),
                event_name: "onafterx".into(),
                element: None,
                skip_on_missing_license: true,
                skip_on_missing_permission: false,
            })
        );
    }

    #[test]
    fn element_present_is_returned() {
        let src = r#"codeunit 50101 Sub
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Pub", 'OnAfterX', 'MyElement', false, false)]
    local procedure OnAfterX()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let attr = &af.objects[0].routines[0].attributes_parsed[0];
        let result = parse_event_subscriber_ir(attr, &af.ir).expect("should parse");
        assert_eq!(result.element, Some("myelement".into()));
    }

    #[test]
    fn missing_optional_args_default_to_false() {
        // Only 4 args: element present, args 4+5 absent → both false.
        let src = r#"codeunit 50102 Sub
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Pub", 'OnAfterX', '')]
    local procedure OnAfterX()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let attr = &af.objects[0].routines[0].attributes_parsed[0];
        let result = parse_event_subscriber_ir(attr, &af.ir).expect("should parse");
        assert!(!result.skip_on_missing_license, "absent arg 4 → false");
        assert!(!result.skip_on_missing_permission, "absent arg 5 → false");
    }

    #[test]
    fn malformed_too_few_args_returns_none() {
        let src = r#"codeunit 50103 Sub
{
    [EventSubscriber(ObjectType::Codeunit)]
    local procedure OnAfterX()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let attr = &af.objects[0].routines[0].attributes_parsed[0];
        assert_eq!(parse_event_subscriber_ir(attr, &af.ir), None);
    }

    #[test]
    fn two_event_subscriber_attributes_both_parsed() {
        let src = r#"codeunit 50104 Sub
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Pub1", 'OnAfterX', '', false, false)]
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Pub2", 'OnAfterY', '', false, false)]
    local procedure MultiSub()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let attrs = &af.objects[0].routines[0].attributes_parsed;
        assert_eq!(
            attrs.len(),
            2,
            "both EventSubscriber attrs attached to the routine"
        );
        let r1 = parse_event_subscriber_ir(&attrs[0], &af.ir).expect("first attr parses");
        let r2 = parse_event_subscriber_ir(&attrs[1], &af.ir).expect("second attr parses");
        assert_eq!(r1.publisher_name, "pub1");
        assert_eq!(r1.event_name, "onafterx");
        assert_eq!(r2.publisher_name, "pub2");
        assert_eq!(r2.event_name, "onaftery");
    }

    // ── is_event_publisher ────────────────────────────────────────────────────

    #[test]
    fn integration_event_attribute_detected() {
        let src = r#"codeunit 50200 Pub
{
    [IntegrationEvent(false, false)]
    procedure OnAfterX()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let r = &af.objects[0].routines[0];
        assert_eq!(is_event_publisher(r), Some(PublisherKind::Integration));
    }

    #[test]
    fn business_event_attribute_detected() {
        let src = r#"codeunit 50201 Pub
{
    [BusinessEvent(false)]
    procedure OnAfterX()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let r = &af.objects[0].routines[0];
        assert_eq!(is_event_publisher(r), Some(PublisherKind::Business));
    }

    #[test]
    fn no_publisher_attribute_returns_none() {
        let src = r#"codeunit 50202 Plain
{
    procedure Plain()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        let r = &af.objects[0].routines[0];
        assert_eq!(is_event_publisher(r), None);
    }

    // ── read_event_subscriber_instance ───────────────────────────────────────

    #[test]
    fn event_subscriber_instance_manual_true() {
        let src = r#"codeunit 50300 Sub
{
    EventSubscriberInstance = Manual;

    procedure OnAfterX()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        assert!(
            read_event_subscriber_instance(&af.objects[0]),
            "Manual → true"
        );
    }

    #[test]
    fn event_subscriber_instance_absent_false() {
        let src = r#"codeunit 50301 Plain
{
    procedure Plain()
    begin
    end;
}"#;
        let af = al_syntax::parse(src);
        assert!(
            !read_event_subscriber_instance(&af.objects[0]),
            "absent → false"
        );
    }
}
