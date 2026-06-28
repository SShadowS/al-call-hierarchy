//! Extract object + routine nodes from one parsed `AlFile`.

use al_syntax::ir::{AlFile, RoutineKind};

use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
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
        for r in &obj.routines {
            routines.push(RoutineNode {
                id: RoutineNodeId {
                    object: obj_id.clone(),
                    name_lc: r.name.to_ascii_lowercase(),
                },
                name: r.name.clone(),
                is_trigger: matches!(r.kind, RoutineKind::Trigger),
                access: Access::from_modifier(r.access_modifier.as_deref()),
                tier,
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
