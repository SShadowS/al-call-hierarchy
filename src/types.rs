//! Core AL object-type enum, shared between the library and binary crate.
//!
//! Kept in the library so both `app_package` and `src/lsp/*` (the LSP surface,
//! e.g. `lsp::custom`'s `al-preview://` URI parsing) can reference the same
//! type without a crate-boundary clash.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Type of AL object
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectType {
    Codeunit,
    Table,
    Page,
    Report,
    Query,
    XmlPort,
    Enum,
    Interface,
    ControlAddIn,
    PageExtension,
    TableExtension,
    EnumExtension,
    PermissionSet,
    PermissionSetExtension,
}

impl TryFrom<&str> for ObjectType {
    type Error = ();

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "codeunit" => Ok(Self::Codeunit),
            "table" => Ok(Self::Table),
            "page" => Ok(Self::Page),
            "report" => Ok(Self::Report),
            "query" => Ok(Self::Query),
            "xmlport" => Ok(Self::XmlPort),
            "enum" => Ok(Self::Enum),
            "interface" => Ok(Self::Interface),
            "controladdin" => Ok(Self::ControlAddIn),
            "pageextension" => Ok(Self::PageExtension),
            "tableextension" => Ok(Self::TableExtension),
            "enumextension" => Ok(Self::EnumExtension),
            "permissionset" => Ok(Self::PermissionSet),
            "permissionsetextension" => Ok(Self::PermissionSetExtension),
            _ => Err(()),
        }
    }
}

impl fmt::Display for ObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codeunit => write!(f, "Codeunit"),
            Self::Table => write!(f, "Table"),
            Self::Page => write!(f, "Page"),
            Self::Report => write!(f, "Report"),
            Self::Query => write!(f, "Query"),
            Self::XmlPort => write!(f, "XmlPort"),
            Self::Enum => write!(f, "Enum"),
            Self::Interface => write!(f, "Interface"),
            Self::ControlAddIn => write!(f, "ControlAddIn"),
            Self::PageExtension => write!(f, "PageExtension"),
            Self::TableExtension => write!(f, "TableExtension"),
            Self::EnumExtension => write!(f, "EnumExtension"),
            Self::PermissionSet => write!(f, "PermissionSet"),
            Self::PermissionSetExtension => write!(f, "PermissionSetExtension"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T3 Task 17: ported verbatim from legacy `src/graph.rs` (deleted this
    // task, which only ever re-exported this type — see the module doc). Without
    // this port, `ObjectType`'s `TryFrom<&str>`/`Display` impls — a surviving,
    // actively-used type — would have had no direct unit test at all.

    #[test]
    fn test_object_type_try_from_valid() {
        assert_eq!(ObjectType::try_from("codeunit"), Ok(ObjectType::Codeunit));
        assert_eq!(ObjectType::try_from("table"), Ok(ObjectType::Table));
        assert_eq!(ObjectType::try_from("page"), Ok(ObjectType::Page));
        assert_eq!(ObjectType::try_from("report"), Ok(ObjectType::Report));
        assert_eq!(ObjectType::try_from("query"), Ok(ObjectType::Query));
        assert_eq!(ObjectType::try_from("xmlport"), Ok(ObjectType::XmlPort));
        assert_eq!(ObjectType::try_from("enum"), Ok(ObjectType::Enum));
        assert_eq!(ObjectType::try_from("interface"), Ok(ObjectType::Interface));
        assert_eq!(
            ObjectType::try_from("controladdin"),
            Ok(ObjectType::ControlAddIn)
        );
        assert_eq!(
            ObjectType::try_from("pageextension"),
            Ok(ObjectType::PageExtension)
        );
        assert_eq!(
            ObjectType::try_from("tableextension"),
            Ok(ObjectType::TableExtension)
        );
        assert_eq!(
            ObjectType::try_from("enumextension"),
            Ok(ObjectType::EnumExtension)
        );
        assert_eq!(
            ObjectType::try_from("permissionset"),
            Ok(ObjectType::PermissionSet)
        );
        assert_eq!(
            ObjectType::try_from("permissionsetextension"),
            Ok(ObjectType::PermissionSetExtension)
        );
    }

    #[test]
    fn test_object_type_try_from_case_insensitive() {
        assert_eq!(ObjectType::try_from("Codeunit"), Ok(ObjectType::Codeunit));
        assert_eq!(ObjectType::try_from("TABLE"), Ok(ObjectType::Table));
    }

    #[test]
    fn test_object_type_try_from_invalid() {
        assert_eq!(ObjectType::try_from("notaobject"), Err(()));
    }

    #[test]
    fn test_object_type_display() {
        assert_eq!(format!("{}", ObjectType::Codeunit), "Codeunit");
        assert_eq!(format!("{}", ObjectType::Table), "Table");
        assert_eq!(format!("{}", ObjectType::Page), "Page");
        assert_eq!(format!("{}", ObjectType::Report), "Report");
        assert_eq!(format!("{}", ObjectType::Query), "Query");
        assert_eq!(format!("{}", ObjectType::XmlPort), "XmlPort");
        assert_eq!(format!("{}", ObjectType::Enum), "Enum");
        assert_eq!(format!("{}", ObjectType::Interface), "Interface");
        assert_eq!(format!("{}", ObjectType::ControlAddIn), "ControlAddIn");
        assert_eq!(format!("{}", ObjectType::PageExtension), "PageExtension");
        assert_eq!(format!("{}", ObjectType::TableExtension), "TableExtension");
        assert_eq!(format!("{}", ObjectType::EnumExtension), "EnumExtension");
        assert_eq!(format!("{}", ObjectType::PermissionSet), "PermissionSet");
        assert_eq!(
            format!("{}", ObjectType::PermissionSetExtension),
            "PermissionSetExtension"
        );
    }
}
