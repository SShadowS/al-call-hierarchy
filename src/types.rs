//! Core AL object-type enum, shared between the library and binary crate.
//!
//! Kept in the library so both `app_package` (lib) and `graph` (binary,
//! re-exports this) can reference the same type without a crate-boundary clash.

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
