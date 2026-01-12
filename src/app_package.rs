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
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
}

/// A method/procedure in an external object
#[derive(Debug, Clone)]
pub struct ExternalMethod {
    pub name: String,
}

/// An object (codeunit, table, etc.) from an external app
#[derive(Debug, Clone)]
pub struct ExternalObject {
    pub name: String,
    pub object_type: ObjectType,
    pub methods: Vec<ExternalMethod>,
}

/// Parsed contents of a .app package
#[derive(Debug)]
pub struct ParsedAppPackage {
    pub metadata: AppMetadata,
    pub objects: Vec<ExternalObject>,
}

/// Intermediate structure for deserializing SymbolReference.json
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolObject {
    name: String,
    #[serde(default)]
    methods: Vec<SymbolMethod>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SymbolMethod {
    name: String,
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
        id: app_node.attribute("Id").unwrap_or_default().to_string(),
        name: app_node.attribute("Name").unwrap_or_default().to_string(),
        publisher: app_node
            .attribute("Publisher")
            .unwrap_or_default()
            .to_string(),
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

    // Helper to convert symbol objects to external objects
    let mut add_objects = |symbol_objects: Vec<SymbolObject>, object_type: ObjectType| {
        for obj in symbol_objects {
            let methods = obj
                .methods
                .into_iter()
                .map(|m| ExternalMethod { name: m.name })
                .collect();

            objects.push(ExternalObject {
                name: obj.name,
                object_type,
                methods,
            });
        }
    };

    add_objects(symbols.tables, ObjectType::Table);
    add_objects(symbols.codeunits, ObjectType::Codeunit);
    add_objects(symbols.pages, ObjectType::Page);
    add_objects(symbols.reports, ObjectType::Report);
    add_objects(symbols.queries, ObjectType::Query);
    add_objects(symbols.xml_ports, ObjectType::XmlPort);
    add_objects(symbols.interfaces, ObjectType::Interface);
    add_objects(symbols.enum_types, ObjectType::Enum);
    add_objects(symbols.control_add_ins, ObjectType::ControlAddIn);
    add_objects(symbols.page_extensions, ObjectType::PageExtension);
    add_objects(symbols.table_extensions, ObjectType::TableExtension);
    add_objects(symbols.enum_extension_types, ObjectType::EnumExtension);
    add_objects(symbols.permission_sets, ObjectType::PermissionSet);
    add_objects(symbols.permission_set_extensions, ObjectType::PermissionSetExtension);

    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_real_app_file() {
        // This test requires the actual test file to exist
        let test_path = Path::new("u:/Git/DO/Cloud/.alpackages/Continia Software_Continia Core_26.0.0.183530.app");
        if !test_path.exists() {
            eprintln!("Skipping test: test file not found");
            return;
        }

        let result = extract_app_package(test_path);
        assert!(result.is_ok(), "Failed to parse app: {:?}", result.err());

        let package = result.unwrap();
        assert_eq!(package.metadata.name, "Continia Core");
        assert_eq!(package.metadata.publisher, "Continia Software");
        assert!(!package.objects.is_empty());

        // Count by type
        let codeunits: Vec<_> = package
            .objects
            .iter()
            .filter(|o| o.object_type == ObjectType::Codeunit)
            .collect();
        assert!(!codeunits.is_empty(), "Should have codeunits");

        println!("Parsed {} objects from {}", package.objects.len(), package.metadata.name);
    }
}
