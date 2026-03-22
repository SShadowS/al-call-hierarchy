//! Tree-sitter AL language bindings

use tree_sitter::Language;

extern "C" {
    fn tree_sitter_al() -> Language;
}

/// Get the tree-sitter AL language
///
/// # Safety
/// This calls into the compiled C code from tree-sitter-al
pub fn language() -> Language {
    unsafe { tree_sitter_al() }
}

/// Tree-sitter queries for extracting AL constructs
pub mod queries {
    /// Query to find procedure and trigger definitions
    pub const DEFINITIONS: &str = r#"
; Procedure definitions
(procedure
  name: [(identifier) (quoted_identifier)] @proc.name)

; Trigger definitions
(trigger_declaration
  name: [(identifier) (quoted_identifier)] @trigger.name)

; Object declarations for context - use object_name field
(codeunit_declaration
  object_name: (_) @codeunit.name)

; Preprocessor-split declaration (files with #if directives)
(preproc_split_declaration
  object_name: (_) @codeunit.name)

(table_declaration
  object_name: (_) @table.name)

(page_declaration
  object_name: (_) @page.name)

(report_declaration
  object_name: (_) @report.name)

(query_declaration
  object_name: (_) @query.name)

(xmlport_declaration
  object_name: (_) @xmlport.name)

(enum_declaration
  object_name: (_) @enum.name)

(interface_declaration
  object_name: (_) @interface.name)

(controladdin_declaration
  object_name: (_) @controladdin.name)

(pageextension_declaration
  object_name: (_) @pageext.name)

(tableextension_declaration
  object_name: (_) @tableext.name)

(enumextension_declaration
  object_name: (_) @enumext.name)

(permissionset_declaration
  object_name: (_) @permissionset.name)

(permissionsetextension_declaration
  object_name: (_) @permissionsetext.name)
"#;

    /// Query to find procedure calls
    pub const CALLS: &str = r#"
; Simple procedure calls: DoSomething()
(call_expression
  function: (identifier) @call.simple) @call

; Method calls: Object.Method() or Rec."Field Name"()
(call_expression
  function: (member_expression
    object: (_) @call.object
    member: (_) @call.method)) @call.member
"#;

    /// Query to find EventSubscriber attributes (V2: attributes are siblings of procedures)
    /// We match attribute_item nodes and resolve the adjacent procedure in Rust code.
    pub const EVENT_SUBSCRIBERS: &str = r#"
(attribute_item
  attribute: (attribute_content
    name: (identifier) @attr.name
    (#eq? @attr.name "EventSubscriber")
    arguments: (attribute_arguments) @attr.args)) @attr.item
"#;

    /// Query to find variable declarations
    pub const VARIABLES: &str = r#"
; Capture all variable declarations - we'll extract name and type manually
(variable_declaration) @var.decl
"#;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_loads() {
        let lang = language();
        assert!(lang.abi_version() > 0);
    }
}
