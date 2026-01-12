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
  name: (name) @proc.name)

; Trigger definitions
(trigger_declaration
  name: (trigger_name) @trigger.name)

; Named triggers (OnInsert, OnModify, etc.)
(named_trigger) @named_trigger.def

; OnRun trigger
(onrun_trigger) @onrun.def

; Object declarations for context - use object_name field
(codeunit_declaration
  object_name: (_) @codeunit.name)

; Preprocessor-split codeunit (files with #if directives)
(preproc_split_codeunit_declaration
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

; Method calls: Object.Method()
(call_expression
  function: (member_expression
    object: (_) @call.object
    property: (_) @call.method)) @call.member

; Field access that might be triggers: Rec.Validate()
(call_expression
  function: (field_access
    record: (_) @call.record
    field: (_) @call.field)) @call.field_access
"#;

    /// Query to find EventSubscriber attributes
    pub const EVENT_SUBSCRIBERS: &str = r#"
; EventSubscriber attribute on procedures
(procedure
  (attribute_item
    (attribute_content
      name: (identifier) @attr.name
      (#eq? @attr.name "EventSubscriber")
      arguments: (attribute_arguments) @attr.args))
  name: (name) @proc.name) @subscriber
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
