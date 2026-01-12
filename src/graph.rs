//! Call graph data structures

use lsp_types::Range;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use string_interner::backend::StringBackend;
use string_interner::{DefaultSymbol, StringInterner};

/// Interned symbol type
pub type Symbol = DefaultSymbol;

/// Shared path reference to reduce memory usage
/// Multiple Definition/CallSite instances can share the same path
pub type SharedPath = Arc<PathBuf>;

/// Index into the call_sites vector
/// Using u32 saves memory vs usize on 64-bit systems
pub type CallSiteIdx = u32;

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

/// Kind of definition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DefinitionKind {
    Procedure,
    Trigger,
    EventSubscriber,
}

impl fmt::Display for DefinitionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Procedure => write!(f, "Procedure"),
            Self::Trigger => write!(f, "Trigger"),
            Self::EventSubscriber => write!(f, "EventSubscriber"),
        }
    }
}

/// A procedure or trigger definition
#[derive(Debug, Clone)]
pub struct Definition {
    /// File containing the definition (shared across multiple definitions)
    pub file: SharedPath,
    /// Range of the definition in the file
    pub range: Range,
    /// Type of containing object
    pub object_type: ObjectType,
    /// Name of containing object (interned)
    pub object_name: Symbol,
    /// Name of the procedure/trigger (interned)
    pub name: Symbol,
    /// Kind of definition
    pub kind: DefinitionKind,
}

/// A call site (where a procedure is called)
#[derive(Debug, Clone)]
pub struct CallSite {
    /// File containing the call (shared across multiple call sites)
    pub file: SharedPath,
    /// Range of the call expression
    pub range: Range,
    /// The procedure containing this call (interned)
    pub caller: Symbol,
    /// Object being called, if qualified (e.g., "CustomerMgt" in CustomerMgt.Create())
    pub callee_object: Option<Symbol>,
    /// Method/procedure being called (interned)
    pub callee_method: Symbol,
}

/// Fully qualified procedure name
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QualifiedName {
    pub object: Symbol,
    pub procedure: Symbol,
}

/// A variable binding (variable name → type)
#[derive(Debug, Clone)]
pub struct VariableBinding {
    /// Variable name (interned)
    pub name: Symbol,
    /// Type name - the object name for Record/Codeunit types (interned)
    pub type_name: Symbol,
    /// Type kind (Record, Codeunit, etc.)
    pub type_kind: Option<String>,
}

/// The call graph index
pub struct CallGraph {
    /// String interner for symbols
    interner: StringInterner<StringBackend>,

    /// Path cache for deduplication - maps PathBuf to SharedPath
    path_cache: HashMap<PathBuf, SharedPath>,

    /// Procedure definitions by qualified name
    definitions: HashMap<QualifiedName, Definition>,

    /// All call sites stored once (indices used in incoming/outgoing maps)
    /// Option allows tombstoning removed entries without shifting indices
    call_sites: Vec<Option<CallSite>>,

    /// File -> call site indices in that file (for incremental removal)
    file_call_sites: HashMap<SharedPath, Vec<CallSiteIdx>>,

    /// Incoming calls: procedure -> indices of call sites that call it
    incoming_calls: HashMap<QualifiedName, Vec<CallSiteIdx>>,

    /// Outgoing calls: procedure -> indices of calls it makes
    outgoing_calls: HashMap<QualifiedName, Vec<CallSiteIdx>>,

    /// File -> definitions in that file (for incremental updates)
    file_definitions: HashMap<SharedPath, Vec<QualifiedName>>,

    /// Object name -> object type mapping
    object_types: HashMap<Symbol, ObjectType>,

    /// Variable bindings: procedure scope -> variable name -> type symbol
    /// Uses QualifiedName for procedure scope, Symbol for variable name
    variable_bindings: HashMap<QualifiedName, SmallVec<[(Symbol, Symbol); 4]>>,

    /// File -> procedures with variables in that file (for cleanup)
    file_variables: HashMap<SharedPath, Vec<QualifiedName>>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self {
            interner: StringInterner::default(),
            path_cache: HashMap::new(),
            definitions: HashMap::new(),
            call_sites: Vec::new(),
            file_call_sites: HashMap::new(),
            incoming_calls: HashMap::new(),
            outgoing_calls: HashMap::new(),
            file_definitions: HashMap::new(),
            object_types: HashMap::new(),
            variable_bindings: HashMap::new(),
            file_variables: HashMap::new(),
        }
    }

    /// Get or create a shared path reference
    /// This deduplicates path storage across multiple definitions and call sites
    pub fn get_shared_path(&mut self, path: &Path) -> SharedPath {
        if let Some(shared) = self.path_cache.get(path) {
            Arc::clone(shared)
        } else {
            let shared = Arc::new(path.to_path_buf());
            self.path_cache.insert(path.to_path_buf(), Arc::clone(&shared));
            shared
        }
    }

    /// Intern a string and return its symbol
    pub fn intern(&mut self, s: &str) -> Symbol {
        self.interner.get_or_intern(s)
    }

    /// Resolve a symbol to its string
    pub fn resolve(&self, sym: Symbol) -> Option<&str> {
        self.interner.resolve(sym)
    }

    /// Look up an existing symbol without interning
    ///
    /// Returns `None` if the string has not been interned.
    pub fn get_symbol(&self, s: &str) -> Option<Symbol> {
        self.interner.get(s)
    }

    /// Register an object type
    pub fn register_object(&mut self, name: Symbol, object_type: ObjectType) {
        self.object_types.insert(name, object_type);
    }

    /// Add a variable binding for a procedure scope
    pub fn add_variable_binding(
        &mut self,
        file: SharedPath,
        procedure_qname: QualifiedName,
        var_name: Symbol,
        type_name: Symbol,
    ) {
        self.variable_bindings
            .entry(procedure_qname)
            .or_default()
            .push((var_name, type_name));

        self.file_variables
            .entry(file)
            .or_default()
            .push(procedure_qname);
    }

    /// Look up a variable's type in a procedure scope
    /// Uses linear search which is fast for small variable counts (typically 1-4)
    pub fn lookup_variable_type(
        &self,
        procedure_qname: &QualifiedName,
        var_name: Symbol,
    ) -> Option<Symbol> {
        self.variable_bindings
            .get(procedure_qname)
            .and_then(|vars| vars.iter().find(|(name, _)| *name == var_name).map(|(_, ty)| *ty))
    }

    /// Add a definition
    pub fn add_definition(&mut self, def: Definition) {
        let qname = QualifiedName {
            object: def.object_name,
            procedure: def.name,
        };

        self.file_definitions
            .entry(def.file.clone())
            .or_default()
            .push(qname);

        self.definitions.insert(qname, def);
    }

    /// Add a call site
    pub fn add_call_site(&mut self, caller_qname: QualifiedName, call: CallSite) {
        // Store call site once and get its index
        let idx = self.call_sites.len() as CallSiteIdx;
        let file = call.file.clone();
        self.call_sites.push(Some(call));

        // Track which file this call site belongs to (for removal)
        self.file_call_sites
            .entry(file)
            .or_default()
            .push(idx);

        // Add index to outgoing calls of the caller
        self.outgoing_calls
            .entry(caller_qname)
            .or_default()
            .push(idx);

        // Try to resolve the callee and add index to incoming calls
        if let Some(callee_qname) = self.resolve_call_by_idx(&caller_qname, idx) {
            self.incoming_calls
                .entry(callee_qname)
                .or_default()
                .push(idx);
        }
    }

    /// Resolve a call to its target definition (by CallSite reference)
    fn resolve_call(&self, caller_qname: &QualifiedName, call: &CallSite) -> Option<QualifiedName> {
        if let Some(obj) = call.callee_object {
            // First, check if obj is a known object name (O(1) lookup)
            if self.object_types.contains_key(&obj) {
                return Some(QualifiedName {
                    object: obj,
                    procedure: call.callee_method,
                });
            }

            // Otherwise, try to resolve obj as a variable name
            if let Some(resolved_type) = self.lookup_variable_type(caller_qname, obj) {
                return Some(QualifiedName {
                    object: resolved_type,
                    procedure: call.callee_method,
                });
            }

            // Fall back to using the object name as-is (may be external)
            Some(QualifiedName {
                object: obj,
                procedure: call.callee_method,
            })
        } else {
            // Unqualified call - resolve to same object as caller
            Some(QualifiedName {
                object: caller_qname.object,
                procedure: call.callee_method,
            })
        }
    }

    /// Resolve a call by its index in call_sites
    fn resolve_call_by_idx(&self, caller_qname: &QualifiedName, idx: CallSiteIdx) -> Option<QualifiedName> {
        let call = self.call_sites.get(idx as usize)?.as_ref()?;
        self.resolve_call(caller_qname, call)
    }

    /// Get a CallSite by its index
    pub fn get_call_site(&self, idx: CallSiteIdx) -> Option<&CallSite> {
        self.call_sites.get(idx as usize).and_then(|opt| opt.as_ref())
    }

    /// Get a definition by qualified name
    pub fn get_definition(&self, qname: &QualifiedName) -> Option<&Definition> {
        self.definitions.get(qname)
    }

    /// Find definition at a file position
    /// Uses file_definitions index for O(1) file lookup, then O(k) where k = definitions in file
    pub fn find_definition_at(&self, file: &Path, line: u32, character: u32) -> Option<&Definition> {
        // Look up the SharedPath from path cache, or find by value in file_definitions
        let shared_path = self.path_cache.get(file)?;
        let qnames = self.file_definitions.get(shared_path)?;

        for qname in qnames {
            if let Some(def) = self.definitions.get(qname) {
                if def.range.start.line <= line
                    && def.range.end.line >= line
                    && (def.range.start.line < line || def.range.start.character <= character)
                    && (def.range.end.line > line || def.range.end.character >= character)
                {
                    return Some(def);
                }
            }
        }
        None
    }

    /// Get incoming calls to a procedure
    pub fn get_incoming_calls(&self, qname: &QualifiedName) -> Vec<&CallSite> {
        self.incoming_calls
            .get(qname)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&idx| self.get_call_site(idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get outgoing calls from a procedure
    pub fn get_outgoing_calls(&self, qname: &QualifiedName) -> Vec<&CallSite> {
        self.outgoing_calls
            .get(qname)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&idx| self.get_call_site(idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Remove all definitions from a file (for incremental update)
    pub fn remove_file(&mut self, file: &Path) {
        // Get the shared path if it exists
        let shared_path = match self.path_cache.get(file) {
            Some(p) => Arc::clone(p),
            None => return, // File was never indexed
        };

        if let Some(qnames) = self.file_definitions.remove(&shared_path) {
            for qname in qnames {
                self.definitions.remove(&qname);
                self.incoming_calls.remove(&qname);
                self.outgoing_calls.remove(&qname);
            }
        }

        // Clean up variable bindings for procedures in this file
        if let Some(proc_qnames) = self.file_variables.remove(&shared_path) {
            for qname in proc_qnames {
                self.variable_bindings.remove(&qname);
            }
        }

        // Tombstone call sites from this file (set to None)
        if let Some(indices) = self.file_call_sites.remove(&shared_path) {
            for idx in indices {
                if let Some(slot) = self.call_sites.get_mut(idx as usize) {
                    *slot = None;
                }
            }
        }

        // Clean up indices that point to removed call sites
        // The filter_map in get_incoming_calls/get_outgoing_calls handles this,
        // but we also remove the indices to save memory
        for calls in self.incoming_calls.values_mut() {
            calls.retain(|&idx| self.call_sites.get(idx as usize).map(|s| s.is_some()).unwrap_or(false));
        }
        for calls in self.outgoing_calls.values_mut() {
            calls.retain(|&idx| self.call_sites.get(idx as usize).map(|s| s.is_some()).unwrap_or(false));
        }

        // Remove from path cache
        self.path_cache.remove(file);
    }

    /// Get count of definitions
    pub fn definition_count(&self) -> usize {
        self.definitions.len()
    }

    /// Get count of call sites (active, non-tombstoned)
    pub fn call_site_count(&self) -> usize {
        self.call_sites.iter().filter(|s| s.is_some()).count()
    }
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Position;

    fn make_range(start_line: u32, end_line: u32) -> Range {
        Range {
            start: Position {
                line: start_line,
                character: 0,
            },
            end: Position {
                line: end_line,
                character: 100,
            },
        }
    }

    // ==================== Interning Tests ====================

    #[test]
    fn test_intern_and_resolve() {
        let mut graph = CallGraph::new();
        let sym = graph.intern("TestSymbol");
        assert_eq!(graph.resolve(sym), Some("TestSymbol"));
    }

    #[test]
    fn test_intern_same_string_returns_same_symbol() {
        let mut graph = CallGraph::new();
        let sym1 = graph.intern("TestSymbol");
        let sym2 = graph.intern("TestSymbol");
        assert_eq!(sym1, sym2);
    }

    #[test]
    fn test_get_symbol_found() {
        let mut graph = CallGraph::new();
        let sym = graph.intern("TestSymbol");
        assert_eq!(graph.get_symbol("TestSymbol"), Some(sym));
    }

    #[test]
    fn test_get_symbol_not_found() {
        let graph = CallGraph::new();
        assert_eq!(graph.get_symbol("NonExistent"), None);
    }

    #[test]
    fn test_get_shared_path_deduplication() {
        let mut graph = CallGraph::new();
        let path1 = graph.get_shared_path(Path::new("test.al"));
        let path2 = graph.get_shared_path(Path::new("test.al"));

        // Should return the same Arc (same pointer)
        assert!(Arc::ptr_eq(&path1, &path2));
    }

    // ==================== Definition Tests ====================

    #[test]
    fn test_add_definition_single() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");
        let proc_name = graph.intern("TestProc");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file,
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc_name,
            kind: DefinitionKind::Procedure,
        });

        let qname = QualifiedName {
            object: obj_name,
            procedure: proc_name,
        };
        let def = graph.get_definition(&qname);
        assert!(def.is_some());
        assert_eq!(def.unwrap().kind, DefinitionKind::Procedure);
    }

    #[test]
    fn test_add_definition_multiple_same_object() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");
        let proc1 = graph.intern("Proc1");
        let proc2 = graph.intern("Proc2");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc1,
            kind: DefinitionKind::Procedure,
        });

        graph.add_definition(Definition {
            file,
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc2,
            kind: DefinitionKind::Trigger,
        });

        assert_eq!(graph.definition_count(), 2);

        let qname1 = QualifiedName {
            object: obj_name,
            procedure: proc1,
        };
        let qname2 = QualifiedName {
            object: obj_name,
            procedure: proc2,
        };

        assert!(graph.get_definition(&qname1).is_some());
        assert!(graph.get_definition(&qname2).is_some());
        assert_eq!(
            graph.get_definition(&qname2).unwrap().kind,
            DefinitionKind::Trigger
        );
    }

    #[test]
    fn test_add_definition_different_objects() {
        let mut graph = CallGraph::new();
        let obj1 = graph.intern("Codeunit1");
        let obj2 = graph.intern("Codeunit2");
        let proc = graph.intern("SameProc");
        let file1 = graph.get_shared_path(Path::new("file1.al"));
        let file2 = graph.get_shared_path(Path::new("file2.al"));

        graph.add_definition(Definition {
            file: file1,
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj1,
            name: proc,
            kind: DefinitionKind::Procedure,
        });

        graph.add_definition(Definition {
            file: file2,
            range: make_range(10, 20),
            object_type: ObjectType::Table,
            object_name: obj2,
            name: proc,
            kind: DefinitionKind::Procedure,
        });

        assert_eq!(graph.definition_count(), 2);

        let qname1 = QualifiedName {
            object: obj1,
            procedure: proc,
        };
        let qname2 = QualifiedName {
            object: obj2,
            procedure: proc,
        };

        assert_eq!(
            graph.get_definition(&qname1).unwrap().object_type,
            ObjectType::Codeunit
        );
        assert_eq!(
            graph.get_definition(&qname2).unwrap().object_type,
            ObjectType::Table
        );
    }

    #[test]
    fn test_get_definition_not_found() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("NonExistent");

        let qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        assert!(graph.get_definition(&qname).is_none());
    }

    #[test]
    fn test_find_definition_at_exact_match() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");
        let proc_name = graph.intern("TestProc");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file: file.clone(),
            range: Range {
                start: Position {
                    line: 10,
                    character: 4,
                },
                end: Position {
                    line: 20,
                    character: 8,
                },
            },
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc_name,
            kind: DefinitionKind::Procedure,
        });

        // Inside range
        assert!(graph.find_definition_at(&file, 15, 10).is_some());

        // At start boundary
        assert!(graph.find_definition_at(&file, 10, 4).is_some());

        // At end boundary
        assert!(graph.find_definition_at(&file, 20, 8).is_some());
    }

    #[test]
    fn test_find_definition_at_no_match() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");
        let proc_name = graph.intern("TestProc");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc_name,
            kind: DefinitionKind::Procedure,
        });

        // Before range
        assert!(graph.find_definition_at(&file, 5, 0).is_none());

        // After range
        assert!(graph.find_definition_at(&file, 25, 0).is_none());
    }

    #[test]
    fn test_find_definition_at_wrong_file() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");
        let proc_name = graph.intern("TestProc");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file,
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc_name,
            kind: DefinitionKind::Procedure,
        });

        let other_file = Path::new("other.al");
        assert!(graph.find_definition_at(other_file, 15, 10).is_none());
    }

    // ==================== CallSite Tests ====================

    #[test]
    fn test_add_call_site_unqualified() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");
        let caller_proc = graph.intern("Caller");
        let callee_proc = graph.intern("Callee");
        let file = graph.get_shared_path(Path::new("test.al"));

        // Register the object and add definitions
        graph.register_object(obj_name, ObjectType::Codeunit);

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: caller_proc,
            kind: DefinitionKind::Procedure,
        });

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: callee_proc,
            kind: DefinitionKind::Procedure,
        });

        let caller_qname = QualifiedName {
            object: obj_name,
            procedure: caller_proc,
        };

        // Add unqualified call (no callee_object)
        graph.add_call_site(
            caller_qname,
            CallSite {
                file,
                range: make_range(15, 15),
                caller: caller_proc,
                callee_object: None,
                callee_method: callee_proc,
            },
        );

        // Check outgoing calls from caller
        let outgoing = graph.get_outgoing_calls(&caller_qname);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].callee_method, callee_proc);

        // Check incoming calls to callee (should resolve to same object)
        let callee_qname = QualifiedName {
            object: obj_name,
            procedure: callee_proc,
        };
        let incoming = graph.get_incoming_calls(&callee_qname);
        assert_eq!(incoming.len(), 1);
    }

    #[test]
    fn test_add_call_site_qualified() {
        let mut graph = CallGraph::new();
        let caller_obj = graph.intern("CallerCodeunit");
        let callee_obj = graph.intern("CalleeCodeunit");
        let caller_proc = graph.intern("CallerProc");
        let callee_proc = graph.intern("CalleeProc");
        let caller_file = graph.get_shared_path(Path::new("caller.al"));
        let callee_file = graph.get_shared_path(Path::new("callee.al"));

        graph.register_object(caller_obj, ObjectType::Codeunit);
        graph.register_object(callee_obj, ObjectType::Codeunit);

        graph.add_definition(Definition {
            file: caller_file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: caller_obj,
            name: caller_proc,
            kind: DefinitionKind::Procedure,
        });

        graph.add_definition(Definition {
            file: callee_file,
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: callee_obj,
            name: callee_proc,
            kind: DefinitionKind::Procedure,
        });

        let caller_qname = QualifiedName {
            object: caller_obj,
            procedure: caller_proc,
        };

        // Add qualified call: CalleeCodeunit.CalleeProc()
        graph.add_call_site(
            caller_qname,
            CallSite {
                file: caller_file,
                range: make_range(15, 15),
                caller: caller_proc,
                callee_object: Some(callee_obj),
                callee_method: callee_proc,
            },
        );

        // Check outgoing calls
        let outgoing = graph.get_outgoing_calls(&caller_qname);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].callee_object, Some(callee_obj));

        // Check incoming calls to callee
        let callee_qname = QualifiedName {
            object: callee_obj,
            procedure: callee_proc,
        };
        let incoming = graph.get_incoming_calls(&callee_qname);
        assert_eq!(incoming.len(), 1);
    }

    #[test]
    fn test_get_incoming_calls_multiple_callers() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let caller1 = graph.intern("Caller1");
        let caller2 = graph.intern("Caller2");
        let callee = graph.intern("Callee");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.register_object(obj, ObjectType::Codeunit);

        // Add all definitions
        for proc in [caller1, caller2, callee] {
            graph.add_definition(Definition {
                file: file.clone(),
                range: make_range(10, 20),
                object_type: ObjectType::Codeunit,
                object_name: obj,
                name: proc,
                kind: DefinitionKind::Procedure,
            });
        }

        // Caller1 calls Callee
        graph.add_call_site(
            QualifiedName {
                object: obj,
                procedure: caller1,
            },
            CallSite {
                file: file.clone(),
                range: make_range(15, 15),
                caller: caller1,
                callee_object: None,
                callee_method: callee,
            },
        );

        // Caller2 calls Callee
        graph.add_call_site(
            QualifiedName {
                object: obj,
                procedure: caller2,
            },
            CallSite {
                file,
                range: make_range(25, 25),
                caller: caller2,
                callee_object: None,
                callee_method: callee,
            },
        );

        let callee_qname = QualifiedName {
            object: obj,
            procedure: callee,
        };
        let incoming = graph.get_incoming_calls(&callee_qname);
        assert_eq!(incoming.len(), 2);
    }

    #[test]
    fn test_get_outgoing_calls_multiple() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let caller = graph.intern("Caller");
        let callee1 = graph.intern("Callee1");
        let callee2 = graph.intern("Callee2");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.register_object(obj, ObjectType::Codeunit);

        for proc in [caller, callee1, callee2] {
            graph.add_definition(Definition {
                file: file.clone(),
                range: make_range(10, 20),
                object_type: ObjectType::Codeunit,
                object_name: obj,
                name: proc,
                kind: DefinitionKind::Procedure,
            });
        }

        let caller_qname = QualifiedName {
            object: obj,
            procedure: caller,
        };

        graph.add_call_site(
            caller_qname,
            CallSite {
                file: file.clone(),
                range: make_range(15, 15),
                caller: caller,
                callee_object: None,
                callee_method: callee1,
            },
        );

        graph.add_call_site(
            caller_qname,
            CallSite {
                file,
                range: make_range(16, 16),
                caller: caller,
                callee_object: None,
                callee_method: callee2,
            },
        );

        let outgoing = graph.get_outgoing_calls(&caller_qname);
        assert_eq!(outgoing.len(), 2);
    }

    #[test]
    fn test_get_incoming_calls_empty() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("TestProc");

        let qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        assert!(graph.get_incoming_calls(&qname).is_empty());
    }

    #[test]
    fn test_get_outgoing_calls_empty() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("TestProc");

        let qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        assert!(graph.get_outgoing_calls(&qname).is_empty());
    }

    // ==================== Variable Binding Tests ====================

    #[test]
    fn test_add_variable_binding() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("TestProc");
        let var_name = graph.intern("Customer");
        let type_name = graph.intern("CustomerTable");
        let file = graph.get_shared_path(Path::new("test.al"));

        let proc_qname = QualifiedName {
            object: obj,
            procedure: proc,
        };

        graph.add_variable_binding(file, proc_qname, var_name, type_name);

        let result = graph.lookup_variable_type(&proc_qname, var_name);
        assert_eq!(result, Some(type_name));
    }

    #[test]
    fn test_lookup_variable_type_not_found_variable() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("TestProc");
        let var_name = graph.intern("Customer");
        let type_name = graph.intern("CustomerTable");
        let unknown_var = graph.intern("Unknown");
        let file = graph.get_shared_path(Path::new("test.al"));

        let proc_qname = QualifiedName {
            object: obj,
            procedure: proc,
        };

        graph.add_variable_binding(file, proc_qname, var_name, type_name);

        assert!(graph.lookup_variable_type(&proc_qname, unknown_var).is_none());
    }

    #[test]
    fn test_lookup_variable_type_not_found_procedure() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("TestProc");
        let other_proc = graph.intern("OtherProc");
        let var_name = graph.intern("Customer");
        let type_name = graph.intern("CustomerTable");
        let file = graph.get_shared_path(Path::new("test.al"));

        let proc_qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        let other_qname = QualifiedName {
            object: obj,
            procedure: other_proc,
        };

        graph.add_variable_binding(file, proc_qname, var_name, type_name);

        assert!(graph.lookup_variable_type(&other_qname, var_name).is_none());
    }

    #[test]
    fn test_call_resolution_via_variable_binding() {
        let mut graph = CallGraph::new();
        let caller_obj = graph.intern("CallerCodeunit");
        let record_table = graph.intern("CustomerTable");
        let caller_proc = graph.intern("CallerProc");
        let callee_proc = graph.intern("Validate");
        let var_name = graph.intern("Cust");
        let caller_file = graph.get_shared_path(Path::new("caller.al"));
        let table_file = graph.get_shared_path(Path::new("table.al"));

        graph.register_object(caller_obj, ObjectType::Codeunit);
        graph.register_object(record_table, ObjectType::Table);

        // Add caller definition
        graph.add_definition(Definition {
            file: caller_file.clone(),
            range: make_range(10, 30),
            object_type: ObjectType::Codeunit,
            object_name: caller_obj,
            name: caller_proc,
            kind: DefinitionKind::Procedure,
        });

        // Add target definition on the table
        graph.add_definition(Definition {
            file: table_file,
            range: make_range(10, 20),
            object_type: ObjectType::Table,
            object_name: record_table,
            name: callee_proc,
            kind: DefinitionKind::Procedure,
        });

        let caller_qname = QualifiedName {
            object: caller_obj,
            procedure: caller_proc,
        };

        // Add variable binding: Cust is of type CustomerTable
        graph.add_variable_binding(caller_file.clone(), caller_qname, var_name, record_table);

        // Add call: Cust.Validate() - var_name is used as callee_object
        graph.add_call_site(
            caller_qname,
            CallSite {
                file: caller_file,
                range: make_range(20, 20),
                caller: caller_proc,
                callee_object: Some(var_name), // Variable name, not object name
                callee_method: callee_proc,
            },
        );

        // The call should resolve to CustomerTable.Validate
        let target_qname = QualifiedName {
            object: record_table,
            procedure: callee_proc,
        };
        let incoming = graph.get_incoming_calls(&target_qname);
        assert_eq!(incoming.len(), 1);
    }

    // ==================== File Removal Tests ====================

    #[test]
    fn test_remove_file_clears_definitions() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("TestProc");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: proc,
            kind: DefinitionKind::Procedure,
        });

        assert_eq!(graph.definition_count(), 1);

        graph.remove_file(Path::new("test.al"));

        assert_eq!(graph.definition_count(), 0);
    }

    #[test]
    fn test_remove_file_clears_call_sites() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let caller = graph.intern("Caller");
        let callee = graph.intern("Callee");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.register_object(obj, ObjectType::Codeunit);

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: caller,
            kind: DefinitionKind::Procedure,
        });

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: callee,
            kind: DefinitionKind::Procedure,
        });

        let caller_qname = QualifiedName {
            object: obj,
            procedure: caller,
        };

        graph.add_call_site(
            caller_qname,
            CallSite {
                file: file.clone(),
                range: make_range(15, 15),
                caller: caller,
                callee_object: None,
                callee_method: callee,
            },
        );

        graph.remove_file(Path::new("test.al"));

        // Both incoming and outgoing should be cleared
        let callee_qname = QualifiedName {
            object: obj,
            procedure: callee,
        };
        assert!(graph.get_incoming_calls(&callee_qname).is_empty());
        assert!(graph.get_outgoing_calls(&caller_qname).is_empty());
    }

    #[test]
    fn test_remove_file_clears_variable_bindings() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCodeunit");
        let proc = graph.intern("TestProc");
        let var_name = graph.intern("Customer");
        let type_name = graph.intern("CustomerTable");

        let proc_qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_variable_binding(file.clone(), proc_qname, var_name, type_name);
        assert!(graph.lookup_variable_type(&proc_qname, var_name).is_some());

        graph.remove_file(Path::new("test.al"));

        assert!(graph.lookup_variable_type(&proc_qname, var_name).is_none());
    }

    #[test]
    fn test_remove_file_preserves_other_files() {
        let mut graph = CallGraph::new();
        let obj1 = graph.intern("Codeunit1");
        let obj2 = graph.intern("Codeunit2");
        let proc = graph.intern("TestProc");
        let file1 = graph.get_shared_path(Path::new("file1.al"));
        let file2 = graph.get_shared_path(Path::new("file2.al"));

        graph.add_definition(Definition {
            file: file1.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj1,
            name: proc,
            kind: DefinitionKind::Procedure,
        });

        graph.add_definition(Definition {
            file: file2.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj2,
            name: proc,
            kind: DefinitionKind::Procedure,
        });

        assert_eq!(graph.definition_count(), 2);

        graph.remove_file(Path::new("file1.al"));

        assert_eq!(graph.definition_count(), 1);

        let qname2 = QualifiedName {
            object: obj2,
            procedure: proc,
        };
        assert!(graph.get_definition(&qname2).is_some());
    }

    #[test]
    fn test_remove_file_clears_cross_file_call_sites() {
        let mut graph = CallGraph::new();
        let obj1 = graph.intern("Codeunit1");
        let obj2 = graph.intern("Codeunit2");
        let caller = graph.intern("Caller");
        let callee = graph.intern("Callee");
        let file1 = graph.get_shared_path(Path::new("file1.al"));
        let file2 = graph.get_shared_path(Path::new("file2.al"));

        graph.register_object(obj1, ObjectType::Codeunit);
        graph.register_object(obj2, ObjectType::Codeunit);

        // Caller in file1
        graph.add_definition(Definition {
            file: file1.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj1,
            name: caller,
            kind: DefinitionKind::Procedure,
        });

        // Callee in file2
        graph.add_definition(Definition {
            file: file2.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj2,
            name: callee,
            kind: DefinitionKind::Procedure,
        });

        let caller_qname = QualifiedName {
            object: obj1,
            procedure: caller,
        };

        // Call from file1 to obj2.Callee
        graph.add_call_site(
            caller_qname,
            CallSite {
                file: file1.clone(),
                range: make_range(15, 15),
                caller: caller,
                callee_object: Some(obj2),
                callee_method: callee,
            },
        );

        let callee_qname = QualifiedName {
            object: obj2,
            procedure: callee,
        };
        assert_eq!(graph.get_incoming_calls(&callee_qname).len(), 1);

        // Remove file1 - the call site should be removed from callee's incoming calls
        graph.remove_file(Path::new("file1.al"));

        assert!(graph.get_incoming_calls(&callee_qname).is_empty());
    }

    // ==================== Object Registration Tests ====================

    #[test]
    fn test_register_object() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");

        graph.register_object(obj_name, ObjectType::Codeunit);

        // Verify by using it in call resolution
        let caller_obj = graph.intern("CallerObj");
        let caller_proc = graph.intern("CallerProc");
        let callee_proc = graph.intern("CalleeProc");
        let test_file = graph.get_shared_path(Path::new("test.al"));
        let caller_file = graph.get_shared_path(Path::new("caller.al"));

        graph.register_object(caller_obj, ObjectType::Codeunit);

        let caller_qname = QualifiedName {
            object: caller_obj,
            procedure: caller_proc,
        };

        graph.add_definition(Definition {
            file: test_file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: callee_proc,
            kind: DefinitionKind::Procedure,
        });

        // Call to TestCodeunit.CalleeProc - should resolve because object is registered
        graph.add_call_site(
            caller_qname,
            CallSite {
                file: caller_file.clone(),
                range: make_range(15, 15),
                caller: caller_proc,
                callee_object: Some(obj_name),
                callee_method: callee_proc,
            },
        );

        let callee_qname = QualifiedName {
            object: obj_name,
            procedure: callee_proc,
        };
        assert_eq!(graph.get_incoming_calls(&callee_qname).len(), 1);
    }
}
