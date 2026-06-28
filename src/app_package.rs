//! Parser for AL .app package files
//!
//! .app files are ZIP archives with a 40-byte NAVX header containing:
//! - NavxManifest.xml: App metadata (ID, name, publisher, version)
//! - SymbolReference.json: All symbol definitions (codeunits, tables, etc.)

use crate::graph::ObjectType;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Size of the NAVX header prepended to .app files
const NAVX_HEADER_SIZE: u64 = 40;

/// App metadata from NavxManifest.xml
#[derive(Debug, Clone)]
pub struct AppMetadata {
    pub name: String,
    pub version: String,
}

/// Kind of method (regular procedure, event publisher, or event subscriber).
/// Derived from the method's attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalMethodKind {
    Procedure,
    IntegrationEvent,
    BusinessEvent,
    InternalEvent,
    EventSubscriber,
}

impl ExternalMethodKind {
    /// True if this method is an event publisher (Integration/Business/Internal).
    #[allow(dead_code)] // predicate kept for future consumers
    pub fn is_publisher(&self) -> bool {
        matches!(
            self,
            Self::IntegrationEvent | Self::BusinessEvent | Self::InternalEvent
        )
    }

    /// Render the attribute tag used in detail strings.
    #[allow(dead_code)] // label helper kept for future consumers
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Procedure => "",
            Self::IntegrationEvent => "[IntegrationEvent]",
            Self::BusinessEvent => "[BusinessEvent]",
            Self::InternalEvent => "[InternalEvent]",
            Self::EventSubscriber => "[EventSubscriber]",
        }
    }
}

/// A method/procedure in an external object.
///
/// Carries the full signature + attribute kind so a client (the Go wrapper)
/// can synthesize an LSP documentSymbol response for dependency objects
/// without needing to read the .dal virtual document.
#[derive(Debug, Clone)]
pub struct ExternalMethod {
    pub name: String,
    pub kind: ExternalMethodKind,
    /// Pre-formatted full signature, e.g.
    /// `OnAfterPostApprovalEntries(var ApprovalEntry: Record "Approval Entry"): Boolean`
    pub signature: String,
    /// True when the method is marked `local` in the source.
    pub is_local: bool,
}

/// An object (codeunit, table, etc.) from an external app.
#[derive(Debug, Clone)]
pub struct ExternalObject {
    pub name: String,
    pub object_type: ObjectType,
    /// AL object id (codeunit number, table number, etc.). 0 if unknown.
    /// Microsoft also emits large negative hash-shaped IDs for synthetic
    /// objects; we accept those verbatim — the wrapper compares as i64.
    pub id: i64,
    pub methods: Vec<ExternalMethod>,
}

/// Parsed contents of a .app package.
#[derive(Debug)]
pub struct ParsedAppPackage {
    pub metadata: AppMetadata,
    pub objects: Vec<ExternalObject>,
}

/// Intermediate structure for deserializing SymbolReference.json.
///
/// Top-level keys are flat in older BC versions; from BC 24+ the
/// codeunits/tables/etc. are nested inside `Namespaces`. We accept both.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolReference {
    #[serde(default)]
    tables: Vec<SymbolObject>,
    #[serde(default)]
    codeunits: Vec<SymbolObject>,
    #[serde(default)]
    pages: Vec<SymbolObject>,
    #[serde(default)]
    reports: Vec<SymbolObject>,
    #[serde(default)]
    queries: Vec<SymbolObject>,
    #[serde(default)]
    xml_ports: Vec<SymbolObject>,
    #[serde(default)]
    interfaces: Vec<SymbolObject>,
    #[serde(default)]
    enum_types: Vec<SymbolObject>,
    #[serde(default)]
    control_add_ins: Vec<SymbolObject>,
    #[serde(default)]
    page_extensions: Vec<SymbolObject>,
    #[serde(default)]
    table_extensions: Vec<SymbolObject>,
    #[serde(default)]
    enum_extension_types: Vec<SymbolObject>,
    #[serde(default)]
    permission_sets: Vec<SymbolObject>,
    #[serde(default)]
    permission_set_extensions: Vec<SymbolObject>,
    #[serde(default)]
    namespaces: Vec<SymbolNamespace>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolObject {
    name: String,
    /// Object IDs in BC are typically positive but Microsoft also emits
    /// large hash-shaped IDs for synthetic objects (which can overflow i32
    /// as negative numbers). Accept i64 and clamp downstream.
    #[serde(default)]
    id: i64,
    #[serde(default)]
    methods: Vec<SymbolMethod>,
    /// Nested objects inside a namespace block. Top-level SymbolReference
    /// also has Namespaces, but in BC 24+ codeunits/tables/etc. are nested
    /// inside namespace nodes. This field allows walking that tree.
    #[serde(default)]
    namespaces: Vec<SymbolNamespace>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct SymbolNamespace {
    #[serde(default)]
    tables: Vec<SymbolObject>,
    #[serde(default)]
    codeunits: Vec<SymbolObject>,
    #[serde(default)]
    pages: Vec<SymbolObject>,
    #[serde(default)]
    reports: Vec<SymbolObject>,
    #[serde(default)]
    queries: Vec<SymbolObject>,
    #[serde(default)]
    xml_ports: Vec<SymbolObject>,
    #[serde(default)]
    interfaces: Vec<SymbolObject>,
    #[serde(default)]
    enum_types: Vec<SymbolObject>,
    #[serde(default)]
    control_add_ins: Vec<SymbolObject>,
    #[serde(default)]
    page_extensions: Vec<SymbolObject>,
    #[serde(default)]
    table_extensions: Vec<SymbolObject>,
    #[serde(default)]
    enum_extension_types: Vec<SymbolObject>,
    #[serde(default)]
    permission_sets: Vec<SymbolObject>,
    #[serde(default)]
    permission_set_extensions: Vec<SymbolObject>,
    #[serde(default)]
    namespaces: Vec<SymbolNamespace>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolMethod {
    name: String,
    #[serde(default)]
    parameters: Vec<SymbolParameter>,
    #[serde(default)]
    attributes: Vec<SymbolMethodAttribute>,
    /// Return type, omitted for void methods.
    #[serde(default, rename = "ReturnTypeDefinition")]
    return_type: Option<SymbolTypeDefinition>,
    /// MethodKind tag from the SymbolReference (Method/Local/Internal/etc.).
    /// Microsoft sometimes emits negative values; accept i64 for safety.
    /// Parsed for completeness; not yet read (future design).
    #[serde(default)]
    #[allow(dead_code)]
    method_kind: Option<i64>,
    /// PascalCase nested object: `{"Local": true}` etc. Optional.
    #[serde(default)]
    properties: Vec<SymbolMethodProperty>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolParameter {
    name: String,
    #[serde(default)]
    is_var: bool,
    #[serde(default)]
    type_definition: Option<SymbolTypeDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolTypeDefinition {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    subtype: Option<SymbolSubtype>,
    /// For length-bound text/code types. i64 because some emitters use -1
    /// as a sentinel for "unbounded".
    #[serde(default)]
    length: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolSubtype {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolMethodAttribute {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolMethodProperty {
    name: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

/// Extract and parse a .app package file
pub fn extract_app_package(path: &Path) -> Result<ParsedAppPackage> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open app file: {}", path.display()))?;

    let mut reader = std::io::BufReader::new(file);

    // Skip the 40-byte NAVX header
    reader
        .seek(SeekFrom::Start(NAVX_HEADER_SIZE))
        .context("Failed to skip NAVX header")?;

    // Open as ZIP archive
    let mut archive =
        zip::ZipArchive::new(reader).context("Failed to read app file as ZIP archive")?;

    // Parse NavxManifest.xml
    let metadata = parse_manifest(&mut archive)?;

    // Parse SymbolReference.json
    let objects = parse_symbols(&mut archive)?;

    Ok(ParsedAppPackage { metadata, objects })
}

/// Parse NavxManifest.xml to extract app metadata
fn parse_manifest<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<AppMetadata> {
    let mut manifest_file = archive
        .by_name("NavxManifest.xml")
        .context("NavxManifest.xml not found in app package")?;

    let mut content = String::new();
    manifest_file
        .read_to_string(&mut content)
        .context("Failed to read NavxManifest.xml")?;

    // Parse XML using roxmltree
    let doc = roxmltree::Document::parse(&content).context("Failed to parse NavxManifest.xml")?;

    // Find the App element
    let app_node = doc
        .descendants()
        .find(|n| n.has_tag_name("App"))
        .context("App element not found in NavxManifest.xml")?;

    Ok(AppMetadata {
        name: app_node.attribute("Name").unwrap_or_default().to_string(),
        version: app_node
            .attribute("Version")
            .unwrap_or_default()
            .to_string(),
    })
}

/// Parse SymbolReference.json to extract object definitions
fn parse_symbols<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Vec<ExternalObject>> {
    let mut symbols_file = archive
        .by_name("SymbolReference.json")
        .context("SymbolReference.json not found in app package")?;

    let mut content = Vec::new();
    symbols_file
        .read_to_end(&mut content)
        .context("Failed to read SymbolReference.json")?;

    // Handle UTF-8 BOM if present
    let json_str = if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
        std::str::from_utf8(&content[3..]).context("Invalid UTF-8 in SymbolReference.json")?
    } else {
        std::str::from_utf8(&content).context("Invalid UTF-8 in SymbolReference.json")?
    };

    // The JSON may have null byte padding after the actual content
    // Use serde_json's StreamDeserializer to only parse the first JSON value
    let mut deserializer = serde_json::Deserializer::from_str(json_str).into_iter();

    let symbols: SymbolReference = deserializer
        .next()
        .context("No JSON content in SymbolReference.json")?
        .context("Failed to parse SymbolReference.json")?;

    let mut objects = Vec::new();
    collect_objects_top(symbols, &mut objects);
    Ok(objects)
}

/// Drain top-level SymbolReference into ExternalObject entries, including
/// any nested Namespaces (BC 24+ stores codeunits inside namespace nodes).
fn collect_objects_top(symbols: SymbolReference, out: &mut Vec<ExternalObject>) {
    push_objects(symbols.tables, ObjectType::Table, out);
    push_objects(symbols.codeunits, ObjectType::Codeunit, out);
    push_objects(symbols.pages, ObjectType::Page, out);
    push_objects(symbols.reports, ObjectType::Report, out);
    push_objects(symbols.queries, ObjectType::Query, out);
    push_objects(symbols.xml_ports, ObjectType::XmlPort, out);
    push_objects(symbols.interfaces, ObjectType::Interface, out);
    push_objects(symbols.enum_types, ObjectType::Enum, out);
    push_objects(symbols.control_add_ins, ObjectType::ControlAddIn, out);
    push_objects(symbols.page_extensions, ObjectType::PageExtension, out);
    push_objects(symbols.table_extensions, ObjectType::TableExtension, out);
    push_objects(symbols.enum_extension_types, ObjectType::EnumExtension, out);
    push_objects(symbols.permission_sets, ObjectType::PermissionSet, out);
    push_objects(
        symbols.permission_set_extensions,
        ObjectType::PermissionSetExtension,
        out,
    );
    for ns in symbols.namespaces {
        collect_objects_ns(ns, out);
    }
}

fn collect_objects_ns(ns: SymbolNamespace, out: &mut Vec<ExternalObject>) {
    push_objects(ns.tables, ObjectType::Table, out);
    push_objects(ns.codeunits, ObjectType::Codeunit, out);
    push_objects(ns.pages, ObjectType::Page, out);
    push_objects(ns.reports, ObjectType::Report, out);
    push_objects(ns.queries, ObjectType::Query, out);
    push_objects(ns.xml_ports, ObjectType::XmlPort, out);
    push_objects(ns.interfaces, ObjectType::Interface, out);
    push_objects(ns.enum_types, ObjectType::Enum, out);
    push_objects(ns.control_add_ins, ObjectType::ControlAddIn, out);
    push_objects(ns.page_extensions, ObjectType::PageExtension, out);
    push_objects(ns.table_extensions, ObjectType::TableExtension, out);
    push_objects(ns.enum_extension_types, ObjectType::EnumExtension, out);
    push_objects(ns.permission_sets, ObjectType::PermissionSet, out);
    push_objects(
        ns.permission_set_extensions,
        ObjectType::PermissionSetExtension,
        out,
    );
    for sub in ns.namespaces {
        collect_objects_ns(sub, out);
    }
}

fn push_objects(objs: Vec<SymbolObject>, object_type: ObjectType, out: &mut Vec<ExternalObject>) {
    for obj in objs {
        // Some object kinds (tables, pages) have inline namespace nodes that
        // contain extension objects. Walk them too so nothing is dropped.
        for ns in obj.namespaces {
            collect_objects_ns(ns, out);
        }

        let methods = obj
            .methods
            .into_iter()
            .map(|m| build_external_method(m, &obj.name))
            .collect();

        out.push(ExternalObject {
            name: obj.name,
            object_type,
            id: obj.id,
            methods,
        });
    }
}

fn build_external_method(m: SymbolMethod, _owner: &str) -> ExternalMethod {
    let kind = classify_method_kind(&m.attributes);
    let is_local = m.properties.iter().any(|p| {
        p.name == "Local"
            && p.value
                .as_ref()
                .and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "True")))
                .unwrap_or(false)
    });
    let signature = format_signature(&m.name, &m.parameters, m.return_type.as_ref(), is_local);
    ExternalMethod {
        name: m.name,
        kind,
        signature,
        is_local,
    }
}

fn classify_method_kind(attributes: &[SymbolMethodAttribute]) -> ExternalMethodKind {
    for attr in attributes {
        match attr.name.as_str() {
            "IntegrationEvent" => return ExternalMethodKind::IntegrationEvent,
            "BusinessEvent" => return ExternalMethodKind::BusinessEvent,
            "InternalEvent" => return ExternalMethodKind::InternalEvent,
            "EventSubscriber" => return ExternalMethodKind::EventSubscriber,
            _ => {}
        }
    }
    ExternalMethodKind::Procedure
}

fn format_signature(
    name: &str,
    parameters: &[SymbolParameter],
    return_type: Option<&SymbolTypeDefinition>,
    is_local: bool,
) -> String {
    let mut out = String::new();
    if is_local {
        out.push_str("local procedure ");
    } else {
        out.push_str("procedure ");
    }
    out.push_str(name);
    out.push('(');
    let mut first = true;
    for p in parameters {
        if !first {
            out.push_str("; ");
        }
        first = false;
        if p.is_var {
            out.push_str("var ");
        }
        out.push_str(&p.name);
        out.push_str(": ");
        out.push_str(&format_type(p.type_definition.as_ref()));
    }
    out.push(')');
    if let Some(t) = return_type {
        let rendered = format_type(Some(t));
        if !rendered.is_empty() && rendered != "?" {
            out.push_str(": ");
            out.push_str(&rendered);
        }
    }
    out
}

fn format_type(td: Option<&SymbolTypeDefinition>) -> String {
    let Some(td) = td else {
        return "?".to_string();
    };
    let name = td.name.as_deref().unwrap_or("?");
    let mut out = name.to_string();
    if let Some(sub) = td.subtype.as_ref().and_then(|s| s.name.as_deref()) {
        // Quote subtype if it has spaces
        if sub.contains(' ') || sub.contains('-') || sub.contains('/') {
            out.push_str(&format!(" \"{}\"", sub));
        } else {
            out.push(' ');
            out.push_str(sub);
        }
    }
    if let Some(len) = td.length
        && len > 0
    {
        out.push_str(&format!("[{}]", len));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_real_app_file() {
        // This test requires the actual test file to exist
        let test_path = Path::new(
            "u:/Git/DO/Cloud/.alpackages/Continia Software_Continia Core_26.0.0.183530.app",
        );
        if !test_path.exists() {
            eprintln!("Skipping test: test file not found");
            return;
        }

        let result = extract_app_package(test_path);
        assert!(result.is_ok(), "Failed to parse app: {:?}", result.err());

        let package = result.unwrap();
        assert_eq!(package.metadata.name, "Continia Core");
        assert!(!package.objects.is_empty());

        // Count by type
        let codeunits: Vec<_> = package
            .objects
            .iter()
            .filter(|o| o.object_type == ObjectType::Codeunit)
            .collect();
        assert!(!codeunits.is_empty(), "Should have codeunits");

        println!(
            "Parsed {} objects from {}",
            package.objects.len(),
            package.metadata.name
        );
    }

    /// Parse Approvals Mgmt. from the real Base Application package and
    /// assert that event-publisher methods are detected with full signatures.
    /// Skipped when the test .app file is not available.
    #[test]
    fn test_parse_approvals_mgmt_events() {
        let test_path = Path::new(
            r"U:\Git\DO.Support-wi-75360\DocumentOutput\.alpackages\Microsoft_Base Application_28.0.46665.47126.app",
        );
        if !test_path.exists() {
            eprintln!("Skipping: Base Application app not present");
            return;
        }

        let pkg = extract_app_package(test_path).expect("parse Base Application");
        let approvals = pkg
            .objects
            .iter()
            .find(|o| o.name == "Approvals Mgmt." && o.object_type == ObjectType::Codeunit)
            .expect("Approvals Mgmt. codeunit");

        // The real Approvals Mgmt. codeunit has ~258 methods including
        // around 137 IntegrationEvent publishers in BC 28. Don't assert
        // exact counts (Microsoft adjusts them across patch releases);
        // do assert the orders of magnitude and that classification works.
        assert!(approvals.methods.len() > 100, "expected many methods");
        let int_events = approvals
            .methods
            .iter()
            .filter(|m| m.kind == ExternalMethodKind::IntegrationEvent)
            .count();
        assert!(int_events > 50, "expected many IntegrationEvents");

        // Pick one event and confirm the signature is populated.
        let any_event = approvals
            .methods
            .iter()
            .find(|m| m.kind == ExternalMethodKind::IntegrationEvent)
            .unwrap();
        assert!(
            any_event.signature.starts_with("procedure ")
                || any_event.signature.starts_with("local procedure ")
        );
        assert!(any_event.signature.contains('('));

        eprintln!(
            "Approvals Mgmt.: {} methods, {} IntegrationEvents",
            approvals.methods.len(),
            int_events
        );
        eprintln!("Sample event: {}", any_event.signature);
    }
}
