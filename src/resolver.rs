//! Call resolution logic

use crate::graph::{CallGraph, CallSite, Definition, QualifiedName};

/// Resolver for matching calls to definitions
pub struct Resolver<'a> {
    graph: &'a CallGraph,
}

impl<'a> Resolver<'a> {
    pub fn new(graph: &'a CallGraph) -> Self {
        Self { graph }
    }

    /// Resolve a call site to its target definition
    pub fn resolve_call(&self, call: &CallSite) -> Option<&Definition> {
        let qname = self.resolve_call_to_qname(call)?;
        self.graph.get_definition(&qname)
    }

    /// Resolve a call site to a qualified name
    pub fn resolve_call_to_qname(&self, call: &CallSite) -> Option<QualifiedName> {
        if let Some(obj) = call.callee_object {
            // Qualified call: Object.Method()
            Some(QualifiedName {
                object: obj,
                procedure: call.callee_method,
            })
        } else {
            // Unqualified call - could be:
            // 1. Local procedure in same object
            // 2. Built-in function
            // For now, we can't resolve without knowing the caller's object
            None
        }
    }

    /// Find all definitions that could match a method name
    pub fn find_matching_definitions(&self, _method_name: &str) -> Vec<&Definition> {
        // This would iterate all definitions and find those with matching procedure name
        // For now, just return empty - this is a placeholder for fuzzy matching
        Vec::new()
    }
}

/// Normalize object names for matching
pub fn normalize_object_name(name: &str) -> String {
    // Remove common prefixes/suffixes, quotes, spaces
    name.trim()
        .trim_matches('"')
        .replace(' ', "")
        .replace('-', "")
        .to_lowercase()
}

/// Check if two object names might refer to the same object
pub fn names_match(name1: &str, name2: &str) -> bool {
    normalize_object_name(name1) == normalize_object_name(name2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_object_name() {
        assert_eq!(normalize_object_name("Customer Mgt."), "customermgt.");
        assert_eq!(normalize_object_name("\"Sales-Post\""), "salespost");
    }

    #[test]
    fn test_names_match() {
        assert!(names_match("Customer Mgt.", "CustomerMgt."));
        assert!(names_match("\"Sales-Post\"", "salespost"));
    }
}
