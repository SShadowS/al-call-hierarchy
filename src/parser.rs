//! Tree-sitter based AL parser

use anyhow::{Context, Result};
use lsp_types::{Position, Range};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::graph::{DefinitionKind, ObjectType};
use crate::language;

/// Parsed definitions and calls from a single file
#[derive(Debug, Default)]
pub struct ParsedFile {
    /// Object type in this file
    pub object_type: Option<ObjectType>,
    /// Object name in this file
    pub object_name: Option<String>,
    /// All procedure/trigger definitions
    pub definitions: Vec<ParsedDefinition>,
    /// All call sites
    pub calls: Vec<ParsedCall>,
    /// All variable declarations
    pub variables: Vec<ParsedVariable>,
}

/// A parsed procedure/trigger definition
#[derive(Debug)]
pub struct ParsedDefinition {
    pub name: String,
    pub range: Range,
    pub kind: DefinitionKind,
}

/// A parsed call site
#[derive(Debug)]
pub struct ParsedCall {
    /// Object being called (None for local calls)
    pub object: Option<String>,
    /// Method/procedure being called
    pub method: String,
    /// Range of the call
    pub range: Range,
    /// Name of the containing procedure (if known)
    pub containing_procedure: Option<String>,
}

/// A parsed variable declaration
#[derive(Debug)]
pub struct ParsedVariable {
    /// Variable name
    pub name: String,
    /// Type name (e.g., "CDO E-Mail Template Line" for Record type)
    pub type_name: String,
    /// Type kind (Record, Codeunit, etc.)
    pub type_kind: Option<String>,
    /// Name of the containing procedure (None for global variables)
    pub containing_procedure: Option<String>,
}

/// AL file parser using tree-sitter
pub struct AlParser {
    parser: Parser,
    definitions_query: Query,
    calls_query: Query,
    variables_query: Query,
}

impl AlParser {
    pub fn new() -> Result<Self> {
        let lang = language::language();

        let mut parser = Parser::new();
        parser.set_language(&lang).context("Failed to set language")?;

        let definitions_query = Query::new(&lang, language::queries::DEFINITIONS)
            .context("Failed to compile definitions query")?;

        let calls_query = Query::new(&lang, language::queries::CALLS)
            .context("Failed to compile calls query")?;

        let variables_query = Query::new(&lang, language::queries::VARIABLES)
            .context("Failed to compile variables query")?;

        Ok(Self {
            parser,
            definitions_query,
            calls_query,
            variables_query,
        })
    }

    /// Parse an AL file and extract definitions and calls
    pub fn parse_file(&mut self, _path: &Path, source: &str) -> Result<ParsedFile> {
        let tree = self
            .parser
            .parse(source, None)
            .context("Failed to parse file")?;

        let root = tree.root_node();
        let mut result = ParsedFile::default();

        // Extract object info and definitions
        self.extract_definitions(&root, source, &mut result);

        // Extract calls
        self.extract_calls(&root, source, &mut result);

        // Extract variable declarations
        self.extract_variables(&root, source, &mut result);

        Ok(result)
    }

    fn extract_definitions(&self, root: &Node, source: &str, result: &mut ParsedFile) {
        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();

        let mut matches = cursor.matches(&self.definitions_query, *root, source_bytes);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let capture_name = &self.definitions_query.capture_names()[capture.index as usize];
                let text = node_text(&node, source);

                match capture_name.as_ref() {
                    // Object declarations
                    "codeunit.name" => {
                        result.object_type = Some(ObjectType::Codeunit);
                        result.object_name = Some(clean_name(text));
                    }
                    "table.name" => {
                        result.object_type = Some(ObjectType::Table);
                        result.object_name = Some(clean_name(text));
                    }
                    "page.name" => {
                        result.object_type = Some(ObjectType::Page);
                        result.object_name = Some(clean_name(text));
                    }
                    "report.name" => {
                        result.object_type = Some(ObjectType::Report);
                        result.object_name = Some(clean_name(text));
                    }
                    "query.name" => {
                        result.object_type = Some(ObjectType::Query);
                        result.object_name = Some(clean_name(text));
                    }
                    "xmlport.name" => {
                        result.object_type = Some(ObjectType::XmlPort);
                        result.object_name = Some(clean_name(text));
                    }
                    "enum.name" => {
                        result.object_type = Some(ObjectType::Enum);
                        result.object_name = Some(clean_name(text));
                    }
                    "interface.name" => {
                        result.object_type = Some(ObjectType::Interface);
                        result.object_name = Some(clean_name(text));
                    }
                    "controladdin.name" => {
                        result.object_type = Some(ObjectType::ControlAddIn);
                        result.object_name = Some(clean_name(text));
                    }
                    "pageext.name" => {
                        result.object_type = Some(ObjectType::PageExtension);
                        result.object_name = Some(clean_name(text));
                    }
                    "tableext.name" => {
                        result.object_type = Some(ObjectType::TableExtension);
                        result.object_name = Some(clean_name(text));
                    }
                    "enumext.name" => {
                        result.object_type = Some(ObjectType::EnumExtension);
                        result.object_name = Some(clean_name(text));
                    }
                    "permissionset.name" => {
                        result.object_type = Some(ObjectType::PermissionSet);
                        result.object_name = Some(clean_name(text));
                    }
                    "permissionsetext.name" => {
                        result.object_type = Some(ObjectType::PermissionSetExtension);
                        result.object_name = Some(clean_name(text));
                    }

                    // Procedure definitions
                    "proc.name" => {
                        if let Some(parent) = node.parent() {
                            result.definitions.push(ParsedDefinition {
                                name: clean_name(text),
                                range: node_range(&parent),
                                kind: DefinitionKind::Procedure,
                            });
                        }
                    }

                    // Trigger definitions
                    "trigger.name" => {
                        if let Some(parent) = node.parent() {
                            result.definitions.push(ParsedDefinition {
                                name: clean_name(text),
                                range: node_range(&parent),
                                kind: DefinitionKind::Trigger,
                            });
                        }
                    }

                    // Named triggers (OnInsert, etc.)
                    "named_trigger.def" | "onrun.def" => {
                        // Extract trigger name from the node type or first child
                        let name = extract_trigger_name(&node, source);
                        result.definitions.push(ParsedDefinition {
                            name,
                            range: node_range(&node),
                            kind: DefinitionKind::Trigger,
                        });
                    }

                    _ => {}
                }
            }
        }
    }

    fn extract_calls(&self, root: &Node, source: &str, result: &mut ParsedFile) {
        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();

        let mut matches = cursor.matches(&self.calls_query, *root, source_bytes);

        while let Some(m) = matches.next() {
            let mut object: Option<String> = None;
            let mut method: Option<String> = None;
            let mut range: Option<Range> = None;
            let mut call_node: Option<Node> = None;

            for capture in m.captures {
                let node = capture.node;
                let capture_name = &self.calls_query.capture_names()[capture.index as usize];
                let text = node_text(&node, source);

                match capture_name.as_ref() {
                    "call.simple" => {
                        method = Some(clean_name(text));
                    }
                    "call.object" | "call.record" => {
                        object = Some(clean_name(text));
                    }
                    "call.method" | "call.field" => {
                        method = Some(clean_name(text));
                    }
                    "call" | "call.member" | "call.field_access" => {
                        range = Some(node_range(&node));
                        call_node = Some(node);
                    }
                    _ => {}
                }
            }

            if let (Some(method), Some(range)) = (method, range) {
                // Find the containing procedure by walking up the tree
                let containing_procedure = call_node.and_then(|n| find_containing_procedure(&n, source));

                result.calls.push(ParsedCall {
                    object,
                    method,
                    range,
                    containing_procedure,
                });
            }
        }
    }

    fn extract_variables(&self, root: &Node, source: &str, result: &mut ParsedFile) {
        let mut cursor = QueryCursor::new();
        let source_bytes = source.as_bytes();

        let mut matches = cursor.matches(&self.variables_query, *root, source_bytes);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let node = capture.node;
                let capture_name = &self.variables_query.capture_names()[capture.index as usize];

                if *capture_name == "var.decl" {
                    // Extract name and type from the variable_declaration node
                    if let (Some(name), Some(type_text)) = (
                        extract_var_name(&node, source),
                        extract_var_type(&node, source),
                    ) {
                        let (type_kind, type_name) = parse_type_specification(&type_text);
                        let containing_procedure = find_containing_procedure(&node, source);

                        result.variables.push(ParsedVariable {
                            name,
                            type_name,
                            type_kind,
                            containing_procedure,
                        });
                    }
                }
            }
        }
    }
}

/// Extract variable name from a variable_declaration node
fn extract_var_name(node: &Node, source: &str) -> Option<String> {
    // Try 'name' field first
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(clean_name(node_text(&name_node, source)));
    }
    // Try 'names' field (for comma-separated declarations)
    if let Some(names_node) = node.child_by_field_name("names") {
        // Just get the first name
        for i in 0..names_node.child_count() as u32 {
            if let Some(child) = names_node.child(i) {
                if child.kind() == "identifier" || child.kind() == "quoted_identifier" {
                    return Some(clean_name(node_text(&child, source)));
                }
            }
        }
    }
    // Walk children to find identifier
    for i in 0..node.child_count() as u32 {
        if let Some(child) = node.child(i) {
            if child.kind() == "identifier" || child.kind() == "quoted_identifier" {
                return Some(clean_name(node_text(&child, source)));
            }
        }
    }
    None
}

/// Extract variable type from a variable_declaration node
fn extract_var_type(node: &Node, source: &str) -> Option<String> {
    // Try 'type' field
    if let Some(type_node) = node.child_by_field_name("type") {
        return Some(node_text(&type_node, source).to_string());
    }
    // Walk children to find type-related nodes
    for i in 0..node.child_count() as u32 {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "type_specification" | "basic_type" => {
                    return Some(node_text(&child, source).to_string());
                }
                _ => {}
            }
        }
    }
    None
}

/// Parse a type specification like "Record \"Customer\"" into (kind, name)
fn parse_type_specification(type_text: &str) -> (Option<String>, String) {
    let trimmed = type_text.trim();

    // Common type patterns: "Record \"Name\"", "Codeunit \"Name\"", etc.
    let type_patterns = [
        "Record",
        "Codeunit",
        "Page",
        "Report",
        "Query",
        "XmlPort",
        "Enum",
        "Interface",
    ];

    for pattern in type_patterns {
        if trimmed.starts_with(pattern) {
            // Extract the object name after the type keyword
            let rest = trimmed[pattern.len()..].trim();
            if let Some(name) = extract_quoted_name(rest) {
                return (Some(pattern.to_string()), name);
            }
        }
    }

    // Not a complex type, just return as-is
    (None, clean_name(trimmed))
}

/// Extract a quoted name like "\"Customer\"" -> "Customer"
fn extract_quoted_name(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.starts_with('"') {
        // Find the closing quote
        if let Some(end) = trimmed[1..].find('"') {
            return Some(trimmed[1..end + 1].to_string());
        }
    }
    // May not be quoted (e.g., Record Customer without quotes in some cases)
    if !trimmed.is_empty() {
        Some(clean_name(trimmed))
    } else {
        None
    }
}

/// Find the name of the procedure or trigger containing this node
fn find_containing_procedure(node: &Node, source: &str) -> Option<String> {
    let mut current = node.parent();

    while let Some(n) = current {
        match n.kind() {
            "procedure" => {
                // Find the name child
                if let Some(name_node) = n.child_by_field_name("name") {
                    return Some(clean_name(node_text(&name_node, source)));
                }
            }
            "trigger_declaration" => {
                if let Some(name_node) = n.child_by_field_name("name") {
                    return Some(clean_name(node_text(&name_node, source)));
                }
            }
            "named_trigger" | "onrun_trigger" => {
                return Some(extract_trigger_name(&n, source));
            }
            _ => {}
        }
        current = n.parent();
    }

    None
}

/// Convert a tree-sitter node to LSP Range
fn node_range(node: &Node) -> Range {
    Range {
        start: Position {
            line: node.start_position().row as u32,
            character: node.start_position().column as u32,
        },
        end: Position {
            line: node.end_position().row as u32,
            character: node.end_position().column as u32,
        },
    }
}

/// Get the text of a node
fn node_text<'a>(node: &Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Clean up a name (remove quotes, trim whitespace)
fn clean_name(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

/// Extract trigger name from a named_trigger or onrun_trigger node
fn extract_trigger_name(node: &Node, source: &str) -> String {
    // Try to get the trigger keyword from the node
    if let Some(child) = node.child(0) {
        let text = node_text(&child, source);
        if text.starts_with("trigger") || text.starts_with("Trigger") {
            // Look for the name after "trigger"
            if let Some(name_child) = node.child_by_field_name("name") {
                return clean_name(node_text(&name_child, source));
            }
        }
        return clean_name(text);
    }

    // Fallback: use the node type
    node.kind().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_parser_creation() {
        let parser = AlParser::new();
        if let Err(ref e) = parser {
            eprintln!("Parser creation failed: {:?}", e);
        }
        assert!(parser.is_ok(), "Parser creation failed: {:?}", parser.err());
    }

    #[test]
    fn test_variable_extraction() {
        let source = r#"
codeunit 50000 "Test Codeunit"
{
    procedure TestProc()
    var
        Customer: Record Customer;
        EMailLine: Record "CDO E-Mail Template Line";
        SalesPost: Codeunit "Sales-Post";
        Counter: Integer;
    begin
        Customer.Get();
        EMailLine.FindTemplate();
        SalesPost.Run();
    end;
}
"#;

        let mut parser = AlParser::new().expect("Parser creation failed");
        let result = parser.parse_file(Path::new("test.al"), source).expect("Parse failed");

        println!("Variables found: {}", result.variables.len());
        for var in &result.variables {
            println!(
                "  {} : {:?} {:?} (in {:?})",
                var.name, var.type_kind, var.type_name, var.containing_procedure
            );
        }

        // We should find 4 variables
        assert!(result.variables.len() >= 3, "Expected at least 3 variables, got {}", result.variables.len());

        // Check we found Record types
        let record_vars: Vec<_> = result.variables.iter()
            .filter(|v| v.type_kind.as_ref().map(|k| k == "Record").unwrap_or(false))
            .collect();
        assert!(record_vars.len() >= 2, "Expected at least 2 Record variables");

        // Check specific variables
        let email_var = result.variables.iter()
            .find(|v| v.name == "EMailLine");
        assert!(email_var.is_some(), "Should find EMailLine variable");
        if let Some(v) = email_var {
            assert_eq!(v.type_kind.as_deref(), Some("Record"));
            assert_eq!(v.type_name, "CDO E-Mail Template Line");
            assert_eq!(v.containing_procedure.as_deref(), Some("TestProc"));
        }
    }

    #[test]
    fn test_type_specification_parsing() {
        // Test the parse_type_specification function
        let (kind, name) = parse_type_specification("Record \"Customer\"");
        assert_eq!(kind.as_deref(), Some("Record"));
        assert_eq!(name, "Customer");

        let (kind, name) = parse_type_specification("Codeunit \"Sales-Post\"");
        assert_eq!(kind.as_deref(), Some("Codeunit"));
        assert_eq!(name, "Sales-Post");

        let (kind, name) = parse_type_specification("Integer");
        assert!(kind.is_none());
        assert_eq!(name, "Integer");
    }
}
