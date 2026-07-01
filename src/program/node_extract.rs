//! Extract object + routine nodes from one parsed `AlFile`.

use al_syntax::ir::{AlFile, Param, RoutineKind};

use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::resolve::edge::{AbiEventKind, AbiRoutineKind};
use crate::program::resolve::event::{
    ParsedSubscriberArgs, PublisherKind, is_event_publisher, parse_event_subscriber_ir,
    read_event_subscriber_instance,
};
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

#[derive(Debug, Clone)]
pub struct ObjectNode {
    pub id: ObjectNodeId,
    pub name: String,
    pub declared_id: Option<i64>,
    pub extends_target: Option<String>,
    pub implements: Vec<String>,
    pub tier: TrustTier,
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
        objects.push(ObjectNode {
            id: obj_id.clone(),
            name: obj.name.clone(),
            declared_id: obj.id,
            extends_target: obj.extends_target.clone(),
            implements: obj.implements.clone(),
            tier,
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
}
