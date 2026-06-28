//! Call graph data structures

use lsp_types::Range;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

// ObjectType is defined in the library crate's `types` module and re-exported
// here so all binary-crate modules can continue using `crate::graph::ObjectType`.
pub use al_call_hierarchy::types::ObjectType;

use crate::protocol::normalize_path;
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
// Several fields (e.g. `object_type`) are carried for future consumers / Debug.
#[allow(dead_code)]
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
    /// Cyclomatic complexity (calculated during parsing)
    pub complexity: u32,
    /// Parameter count
    pub parameter_count: u32,
}

/// Source of an external definition (from a .app package)
#[derive(Debug, Clone, Copy)]
// `app_version` carried for future consumers / Debug.
#[allow(dead_code)]
pub struct ExternalSource {
    /// App name (interned)
    pub app_name: Symbol,
    /// App version (interned)
    pub app_version: Symbol,
}

/// An external definition from a .app package (no file/range available)
#[derive(Debug, Clone)]
// `object_type` / `kind` carried for future consumers / Debug.
#[allow(dead_code)]
pub struct ExternalDefinition {
    /// Source app metadata
    pub source: ExternalSource,
    /// Type of containing object
    pub object_type: ObjectType,
    /// Name of containing object (interned)
    pub object_name: Symbol,
    /// Name of the procedure/trigger (interned)
    pub name: Symbol,
    /// Kind of definition
    pub kind: DefinitionKind,
}

/// Kind of a dependency-object method (publisher / subscriber / regular procedure).
///
/// Parallel to `crate::app_package::ExternalMethodKind` but kept here so the
/// graph module doesn't depend on `app_package` types externally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyMethodKind {
    Procedure,
    IntegrationEvent,
    BusinessEvent,
    InternalEvent,
    EventSubscriber,
}

impl DependencyMethodKind {
    // Classification/label helpers kept for future consumers (dependency doc symbols).
    #[allow(dead_code)]
    pub fn is_publisher(&self) -> bool {
        matches!(
            self,
            Self::IntegrationEvent | Self::BusinessEvent | Self::InternalEvent
        )
    }

    #[allow(dead_code)]
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

/// A method exposed by a dependency object (suitable for documentSymbol response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyMethod {
    pub name: String,
    pub kind: DependencyMethodKind,
    /// Pre-formatted signature, e.g. `procedure Foo(var Bar: Record "Customer"): Boolean`.
    pub signature: String,
    pub is_local: bool,
}

/// An object from a dependency .app package, captured with enough detail to
/// synthesize an LSP documentSymbol response without involving the AL LSP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyObject {
    pub app_name: String,
    pub app_version: String,
    pub object_type: ObjectType,
    pub object_id: i64,
    pub object_name: String,
    pub methods: Vec<DependencyMethod>,
}

/// Lookup key for `CallGraph::dependency_objects`. Case-insensitive on
/// `app_name` and `object_name` since AL is case-insensitive and the AL LSP's
/// al-preview URIs use whatever casing the user typed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DependencyKey {
    pub app_name_lc: String,
    pub object_type: ObjectType,
    pub object_name_lc: String,
}

impl DependencyKey {
    pub fn new(app_name: &str, object_type: ObjectType, object_name: &str) -> Self {
        Self {
            app_name_lc: app_name.to_lowercase(),
            object_type,
            object_name_lc: object_name.to_lowercase(),
        }
    }
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
// Not yet constructed — future design (variable-type resolution).
#[allow(dead_code)]
pub struct VariableBinding {
    /// Variable name (interned)
    pub name: Symbol,
    /// Type name - the object name for Record/Codeunit types (interned)
    pub type_name: Symbol,
    /// Type kind (Record, Codeunit, etc.)
    pub type_kind: Option<String>,
}

/// An event subscription linking a subscriber procedure to a publisher event
#[derive(Debug, Clone)]
// `publisher_object_type` carried for future consumers / Debug.
#[allow(dead_code)]
pub struct EventSubscription {
    /// The subscriber procedure (the one with [EventSubscriber] attribute)
    pub subscriber: QualifiedName,
    /// File containing the subscriber
    pub file: SharedPath,
    /// Range of the subscriber procedure
    pub range: Range,
    /// Object type of the publisher (e.g., Codeunit)
    pub publisher_object_type: Option<ObjectType>,
    /// Name of the publisher object (e.g., "Sales-Post")
    pub publisher_object: Symbol,
    /// Name of the event being subscribed to (e.g., "OnBeforePostSalesDoc")
    pub publisher_event: Symbol,
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

    /// External definitions from .app packages (no file/range)
    external_definitions: HashMap<QualifiedName, ExternalDefinition>,

    /// External object types (from .app packages)
    external_object_types: HashMap<Symbol, ObjectType>,

    /// Event subscriptions: maps publisher qualified name to list of subscribers
    /// Key is (publisher_object, publisher_event), value is list of subscriptions
    event_subscriptions: HashMap<QualifiedName, Vec<EventSubscription>>,

    /// File -> event subscriptions in that file (for incremental removal)
    file_event_subscriptions: HashMap<SharedPath, Vec<QualifiedName>>,

    /// Rich per-object dependency index built from .app SymbolReference.json.
    /// Lets the wrapper synthesize documentSymbol responses for al-preview:/
    /// URIs without going through the AL LSP.
    dependency_objects: HashMap<DependencyKey, DependencyObject>,

    /// Per-file event publishers (procedures with [IntegrationEvent],
    /// [BusinessEvent], or [InternalEvent]) discovered in workspace .al files.
    /// Used to overlay event-kind tagging on AL LSP's documentSymbol response
    /// for local files. Invalidated/repopulated by the file watcher.
    local_event_publishers: HashMap<SharedPath, Vec<LocalEventPublisher>>,

    /// Procedures that are invoked implicitly by a framework rather than by a
    /// direct call, so they must never be reported as unused (issue #20):
    /// event publishers ([IntegrationEvent]/[BusinessEvent]/[InternalEvent]),
    /// test methods ([Test]) and test handlers ([ConfirmHandler], ...).
    /// EventSubscriber procedures are excluded separately via DefinitionKind.
    /// Entries are cleared per-file in remove_file alongside the definitions.
    implicitly_invoked: HashSet<QualifiedName>,
}

/// An event publisher procedure detected in a workspace .al file.
/// Mirrors `parser::ParsedEventPublisher` but lives in the graph so it can be
/// queried from request handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalEventPublisher {
    pub name: String,
    pub range: Range,
    pub selection_range: Range,
    pub kind: LocalEventPublisherKind,
    pub is_local: bool,
    pub signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// The `Event` suffix is the AL domain term, not noise.
#[allow(clippy::enum_variant_names)]
pub enum LocalEventPublisherKind {
    IntegrationEvent,
    BusinessEvent,
    InternalEvent,
}

impl LocalEventPublisherKind {
    #[allow(dead_code)] // label helper kept for future consumers
    pub fn tag(&self) -> &'static str {
        match self {
            Self::IntegrationEvent => "[IntegrationEvent]",
            Self::BusinessEvent => "[BusinessEvent]",
            Self::InternalEvent => "[InternalEvent]",
        }
    }
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
            external_definitions: HashMap::new(),
            external_object_types: HashMap::new(),
            event_subscriptions: HashMap::new(),
            file_event_subscriptions: HashMap::new(),
            dependency_objects: HashMap::new(),
            local_event_publishers: HashMap::new(),
            implicitly_invoked: HashSet::new(),
        }
    }

    /// Mark a procedure as implicitly invoked by a framework (event publisher,
    /// test method, or test handler) so it is excluded from unused-procedure
    /// diagnostics. Cleared per-file by remove_file.
    pub fn mark_implicitly_invoked(&mut self, qname: QualifiedName) {
        self.implicitly_invoked.insert(qname);
    }

    /// Replace the event publisher list for a file (called once per parse).
    pub fn set_local_event_publishers(
        &mut self,
        file: SharedPath,
        publishers: Vec<LocalEventPublisher>,
    ) {
        if publishers.is_empty() {
            self.local_event_publishers.remove(&file);
        } else {
            self.local_event_publishers.insert(file, publishers);
        }
    }

    /// Return event publishers detected in a file. Empty when none.
    pub fn get_local_event_publishers(&self, file: &Path) -> &[LocalEventPublisher] {
        let normalized = normalize_path(file);
        let shared_path = match self.path_cache.get(&normalized) {
            Some(p) => p,
            None => return &[],
        };
        self.local_event_publishers
            .get(shared_path)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Insert or replace a rich dependency object (built from a .app file).
    pub fn add_dependency_object(&mut self, obj: DependencyObject) {
        let key = DependencyKey::new(&obj.app_name, obj.object_type, &obj.object_name);
        self.dependency_objects.insert(key, obj);
    }

    /// Look up a dependency object by (app, type, name). Case-insensitive.
    pub fn get_dependency_object(
        &self,
        app_name: &str,
        object_type: ObjectType,
        object_name: &str,
    ) -> Option<&DependencyObject> {
        let key = DependencyKey::new(app_name, object_type, object_name);
        self.dependency_objects.get(&key)
    }

    /// Look up a dependency object by (type, name) only, scanning all apps.
    /// Useful when the URI doesn't include the app, or when the caller has
    /// approximate naming.
    pub fn find_dependency_object_by_type_name(
        &self,
        object_type: ObjectType,
        object_name: &str,
    ) -> Option<&DependencyObject> {
        let name_lc = object_name.to_lowercase();
        self.dependency_objects
            .values()
            .find(|o| o.object_type == object_type && o.object_name.to_lowercase() == name_lc)
    }

    #[allow(dead_code)] // accessor kept for future consumers / diagnostics
    pub fn dependency_object_count(&self) -> usize {
        self.dependency_objects.len()
    }

    /// Get or create a shared path reference
    /// This deduplicates path storage across multiple definitions and call sites.
    /// Paths are normalized (lowercased on Windows) for case-insensitive matching.
    pub fn get_shared_path(&mut self, path: &Path) -> SharedPath {
        let normalized = normalize_path(path);
        if let Some(shared) = self.path_cache.get(&normalized) {
            Arc::clone(shared)
        } else {
            let shared = Arc::new(normalized.clone());
            self.path_cache.insert(normalized, Arc::clone(&shared));
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

    /// Register an external object type (from .app package)
    pub fn register_external_object(&mut self, name: Symbol, object_type: ObjectType) {
        self.external_object_types.insert(name, object_type);
    }

    /// Add an external definition (from .app package)
    pub fn add_external_definition(&mut self, def: ExternalDefinition) {
        let qname = QualifiedName {
            object: def.object_name,
            procedure: def.name,
        };
        self.external_definitions.insert(qname, def);
    }

    /// Get an external definition by qualified name
    pub fn get_external_definition(&self, qname: &QualifiedName) -> Option<&ExternalDefinition> {
        self.external_definitions.get(qname)
    }

    /// Get count of external definitions
    pub fn external_definition_count(&self) -> usize {
        self.external_definitions.len()
    }

    /// Add an event subscription
    pub fn add_event_subscription(&mut self, subscription: EventSubscription) {
        let publisher_qname = QualifiedName {
            object: subscription.publisher_object,
            procedure: subscription.publisher_event,
        };

        // Track for file-based cleanup
        self.file_event_subscriptions
            .entry(subscription.file.clone())
            .or_default()
            .push(publisher_qname);

        // Add to event subscriptions index
        self.event_subscriptions
            .entry(publisher_qname)
            .or_default()
            .push(subscription);
    }

    /// Get all event subscribers for a given publisher event
    /// Returns subscriptions where the publisher_object and publisher_event match the given qname
    pub fn get_event_subscribers(&self, qname: &QualifiedName) -> Vec<&EventSubscription> {
        self.event_subscriptions
            .get(qname)
            .map(|subs| subs.iter().collect())
            .unwrap_or_default()
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
            .and_then(|vars| {
                vars.iter()
                    .find(|(name, _)| *name == var_name)
                    .map(|(_, ty)| *ty)
            })
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
        self.file_call_sites.entry(file).or_default().push(idx);

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
            // First, check if obj is a known local object name (O(1) lookup)
            if self.object_types.contains_key(&obj) {
                return Some(QualifiedName {
                    object: obj,
                    procedure: call.callee_method,
                });
            }

            // Check if obj is a known external object name (from .app packages)
            if self.external_object_types.contains_key(&obj) {
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

            // Fall back to using the object name as-is (may be unresolved external).
            // This is a resolution miss: the qualified callee object is neither a
            // workspace object, an .app dependency, nor a typed local variable.
            #[cfg(feature = "telemetry")]
            {
                use crate::telemetry::events::{
                    CallPattern, CalleeSource, CallerContext, ObjectType as TelObjectType,
                    ResolutionFailure,
                };
                use crate::telemetry::{CallContext, record_resolution_miss};
                let object_name = self.interner.resolve(obj).unwrap_or("");
                let procedure_name = self.interner.resolve(call.callee_method).unwrap_or("");
                record_resolution_miss(&CallContext {
                    failure: ResolutionFailure::ObjectNotFound,
                    call_pattern: CallPattern::Qualified,
                    callee_object_type: None,
                    callee_source: CalleeSource::Unknown,
                    caller_object_type: TelObjectType::Other,
                    caller_context: CallerContext::Procedure,
                    callee_object_name: Some(object_name),
                    callee_procedure_name: procedure_name,
                    arg_count: 0,
                    ts_node_path: "",
                });
            }
            Some(QualifiedName {
                object: obj,
                procedure: call.callee_method,
            })
        } else {
            // Unqualified call - resolve to same object as caller. If no local
            // definition exists for this (caller_object, callee_method) pair, the
            // call is unresolved (e.g. references an unknown helper or a method
            // on an implicit context like a Page record we haven't inferred).
            #[cfg(feature = "telemetry")]
            {
                let qname = QualifiedName {
                    object: caller_qname.object,
                    procedure: call.callee_method,
                };
                if !self.definitions.contains_key(&qname) {
                    use crate::telemetry::events::{
                        CallPattern, CalleeSource, CallerContext, ObjectType as TelObjectType,
                        ResolutionFailure,
                    };
                    use crate::telemetry::{CallContext, record_resolution_miss};
                    let procedure_name = self.interner.resolve(call.callee_method).unwrap_or("");
                    record_resolution_miss(&CallContext {
                        failure: ResolutionFailure::UnresolvedUnqualified,
                        call_pattern: CallPattern::Unqualified,
                        callee_object_type: None,
                        callee_source: CalleeSource::Workspace,
                        caller_object_type: TelObjectType::Other,
                        caller_context: CallerContext::Procedure,
                        callee_object_name: None,
                        callee_procedure_name: procedure_name,
                        arg_count: 0,
                        ts_node_path: "",
                    });
                }
            }
            Some(QualifiedName {
                object: caller_qname.object,
                procedure: call.callee_method,
            })
        }
    }

    /// Resolve a call by its index in call_sites
    fn resolve_call_by_idx(
        &self,
        caller_qname: &QualifiedName,
        idx: CallSiteIdx,
    ) -> Option<QualifiedName> {
        let call = self.call_sites.get(idx as usize)?.as_ref()?;
        self.resolve_call(caller_qname, call)
    }

    /// Get a CallSite by its index
    pub fn get_call_site(&self, idx: CallSiteIdx) -> Option<&CallSite> {
        self.call_sites
            .get(idx as usize)
            .and_then(|opt| opt.as_ref())
    }

    /// Get a definition by qualified name
    pub fn get_definition(&self, qname: &QualifiedName) -> Option<&Definition> {
        self.definitions.get(qname)
    }

    /// Find definition at a file position
    /// Uses file_definitions index for O(1) file lookup, then O(k) where k = definitions in file
    pub fn find_definition_at(
        &self,
        file: &Path,
        line: u32,
        character: u32,
    ) -> Option<&Definition> {
        // Look up the SharedPath from path cache (normalized for case-insensitive matching)
        let normalized = normalize_path(file);
        let shared_path = self.path_cache.get(&normalized)?;
        let qnames = self.file_definitions.get(shared_path)?;

        for qname in qnames {
            if let Some(def) = self.definitions.get(qname)
                && def.range.start.line <= line
                && def.range.end.line >= line
                && (def.range.start.line < line || def.range.start.character <= character)
                && (def.range.end.line > line || def.range.end.character >= character)
            {
                return Some(def);
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
        // Get the shared path if it exists (normalized for case-insensitive matching)
        let normalized = normalize_path(file);
        let shared_path = match self.path_cache.get(&normalized) {
            Some(p) => Arc::clone(p),
            None => return, // File was never indexed
        };

        if let Some(qnames) = self.file_definitions.remove(&shared_path) {
            for qname in qnames {
                self.definitions.remove(&qname);
                self.incoming_calls.remove(&qname);
                self.outgoing_calls.remove(&qname);
                self.implicitly_invoked.remove(&qname);
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
            calls.retain(|&idx| {
                self.call_sites
                    .get(idx as usize)
                    .map(|s| s.is_some())
                    .unwrap_or(false)
            });
        }
        for calls in self.outgoing_calls.values_mut() {
            calls.retain(|&idx| {
                self.call_sites
                    .get(idx as usize)
                    .map(|s| s.is_some())
                    .unwrap_or(false)
            });
        }

        // Drop any cached event publishers for this file.
        self.local_event_publishers.remove(&shared_path);

        // Clean up event subscriptions from this file
        if let Some(publisher_qnames) = self.file_event_subscriptions.remove(&shared_path) {
            for qname in publisher_qnames {
                if let Some(subs) = self.event_subscriptions.get_mut(&qname) {
                    subs.retain(|sub| sub.file != shared_path);
                    if subs.is_empty() {
                        self.event_subscriptions.remove(&qname);
                    }
                }
            }
        }

        // Remove from path cache
        self.path_cache.remove(&normalized);
    }

    /// Get count of definitions
    pub fn definition_count(&self) -> usize {
        self.definitions.len()
    }

    /// Get count of call sites (active, non-tombstoned)
    pub fn call_site_count(&self) -> usize {
        self.call_sites.iter().filter(|s| s.is_some()).count()
    }

    /// Get all definitions in a specific file
    /// Returns an iterator over definitions for Code Lens support
    pub fn get_definitions_in_file(&self, file: &Path) -> Vec<&Definition> {
        let normalized = normalize_path(file);
        let shared_path = match self.path_cache.get(&normalized) {
            Some(p) => p,
            None => return vec![],
        };

        self.file_definitions
            .get(shared_path)
            .map(|qnames| {
                qnames
                    .iter()
                    .filter_map(|qname| self.definitions.get(qname))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the count of incoming calls for a procedure
    pub fn get_incoming_call_count(&self, qname: &QualifiedName) -> usize {
        // Count direct calls
        let direct_calls = self
            .incoming_calls
            .get(qname)
            .map(|indices| {
                indices
                    .iter()
                    .filter(|&&idx| self.get_call_site(idx).is_some())
                    .count()
            })
            .unwrap_or(0);

        // Count event subscribers (if this is a trigger/event)
        let subscriber_count = self
            .event_subscriptions
            .get(qname)
            .map(|subs| subs.len())
            .unwrap_or(0);

        direct_calls + subscriber_count
    }

    /// Get all unused procedures (procedures with no incoming calls)
    /// Excludes triggers and event subscribers (by DefinitionKind), plus
    /// procedures invoked implicitly by a framework — event publishers, test
    /// methods and test handlers (tracked in implicitly_invoked) — since none
    /// of these are reached through a direct call (issue #20).
    pub fn get_unused_procedures(&self) -> Vec<(&QualifiedName, &Definition)> {
        self.definitions
            .iter()
            .filter(|(qname, def)| {
                // Only check procedures (not triggers or event subscribers)
                def.kind == DefinitionKind::Procedure &&
                // Skip framework-invoked procedures (publishers/tests/handlers)
                !self.implicitly_invoked.contains(qname) &&
                // Check if there are no incoming calls
                self.get_incoming_call_count(qname) == 0
            })
            .collect()
    }

    /// Iterate over all definitions
    pub fn iter_definitions(&self) -> impl Iterator<Item = (&QualifiedName, &Definition)> {
        self.definitions.iter()
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
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file,
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc2,
            kind: DefinitionKind::Trigger,
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file: file2,
            range: make_range(10, 20),
            object_type: ObjectType::Table,
            object_name: obj2,
            name: proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
        });

        let other_file = Path::new("other.al");
        assert!(graph.find_definition_at(other_file, 15, 10).is_none());
    }

    #[test]
    fn test_get_shared_path_case_insensitive_on_windows() {
        let mut graph = CallGraph::new();

        // On Windows, differently-cased paths should map to the same SharedPath
        let path1 = graph.get_shared_path(Path::new("C:\\Git\\Project\\AL\\File.al"));
        let path2 = graph.get_shared_path(Path::new("C:\\Git\\Project\\al\\file.al"));

        #[cfg(windows)]
        assert!(
            Arc::ptr_eq(&path1, &path2),
            "Same file with different case should deduplicate on Windows"
        );
        #[cfg(not(windows))]
        assert!(
            !Arc::ptr_eq(&path1, &path2),
            "Different case paths are distinct on non-Windows"
        );
    }

    #[test]
    fn test_find_definition_at_case_insensitive_on_windows() {
        let mut graph = CallGraph::new();
        let obj_name = graph.intern("TestCodeunit");
        let proc_name = graph.intern("TestProc");
        // Index with one casing
        let file = graph.get_shared_path(Path::new("C:\\Git\\Project\\AL\\Codeunit\\File.al"));

        graph.add_definition(Definition {
            file,
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: proc_name,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        // Look up with different casing (simulating URI-derived path vs WalkDir path)
        let lookup_path = Path::new("C:\\Git\\Project\\al\\codeunit\\file.al");
        #[cfg(windows)]
        assert!(
            graph.find_definition_at(lookup_path, 15, 10).is_some(),
            "Should find definition regardless of path case on Windows"
        );
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
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj_name,
            name: callee_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file: callee_file,
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: callee_obj,
            name: callee_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
                complexity: 0,
                parameter_count: 0,
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
                complexity: 0,
                parameter_count: 0,
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
                caller,
                callee_object: None,
                callee_method: callee1,
            },
        );

        graph.add_call_site(
            caller_qname,
            CallSite {
                file,
                range: make_range(16, 16),
                caller,
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

        assert!(
            graph
                .lookup_variable_type(&proc_qname, unknown_var)
                .is_none()
        );
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
            complexity: 0,
            parameter_count: 0,
        });

        // Add target definition on the table
        graph.add_definition(Definition {
            file: table_file,
            range: make_range(10, 20),
            object_type: ObjectType::Table,
            object_name: record_table,
            name: callee_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: callee,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
                caller,
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
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file: file2.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj2,
            name: proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
            complexity: 0,
            parameter_count: 0,
        });

        // Callee in file2
        graph.add_definition(Definition {
            file: file2.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj2,
            name: callee,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
                caller,
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
            complexity: 0,
            parameter_count: 0,
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

    // ==================== ObjectType TryFrom Tests ====================

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

    // ==================== ObjectType Display Tests ====================

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

    // ==================== DefinitionKind Display Tests ====================

    #[test]
    fn test_definition_kind_display() {
        assert_eq!(format!("{}", DefinitionKind::Procedure), "Procedure");
        assert_eq!(format!("{}", DefinitionKind::Trigger), "Trigger");
        assert_eq!(
            format!("{}", DefinitionKind::EventSubscriber),
            "EventSubscriber"
        );
    }

    // ==================== External Definition Tests ====================

    #[test]
    fn test_external_definitions() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("ExternalCodeunit");
        let proc = graph.intern("ExternalProc");
        let app_name = graph.intern("TestApp");
        let app_version = graph.intern("1.0.0");

        graph.register_external_object(obj, ObjectType::Codeunit);

        graph.add_external_definition(ExternalDefinition {
            source: ExternalSource {
                app_name,
                app_version,
            },
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: proc,
            kind: DefinitionKind::Procedure,
        });

        assert_eq!(graph.external_definition_count(), 1);

        let qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        let ext_def = graph.get_external_definition(&qname);
        assert!(ext_def.is_some());
        assert_eq!(ext_def.unwrap().object_type, ObjectType::Codeunit);
    }

    #[test]
    fn test_external_definition_not_found() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("Missing");
        let proc = graph.intern("MissingProc");
        let qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        assert!(graph.get_external_definition(&qname).is_none());
        assert_eq!(graph.external_definition_count(), 0);
    }

    // ==================== Event Subscription Tests ====================

    #[test]
    fn test_event_subscription_add_and_retrieve() {
        let mut graph = CallGraph::new();
        let subscriber_obj = graph.intern("SubscriberCU");
        let subscriber_proc = graph.intern("OnBeforePost");
        let publisher_obj = graph.intern("PublisherCU");
        let publisher_event = graph.intern("OnPostDocument");
        let file = graph.get_shared_path(Path::new("subscriber.al"));

        let subscription = EventSubscription {
            subscriber: QualifiedName {
                object: subscriber_obj,
                procedure: subscriber_proc,
            },
            file: file.clone(),
            range: make_range(10, 20),
            publisher_object_type: Some(ObjectType::Codeunit),
            publisher_object: publisher_obj,
            publisher_event,
        };

        graph.add_event_subscription(subscription);

        let publisher_qname = QualifiedName {
            object: publisher_obj,
            procedure: publisher_event,
        };
        let subscribers = graph.get_event_subscribers(&publisher_qname);
        assert_eq!(subscribers.len(), 1);
        assert_eq!(subscribers[0].subscriber.object, subscriber_obj);
    }

    #[test]
    fn test_event_subscription_no_subscribers() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("SomeObj");
        let proc = graph.intern("SomeEvent");
        let qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        assert!(graph.get_event_subscribers(&qname).is_empty());
    }

    #[test]
    fn test_remove_file_clears_event_subscriptions() {
        let mut graph = CallGraph::new();
        let subscriber_obj = graph.intern("SubscriberCU");
        let subscriber_proc = graph.intern("OnBeforePost");
        let publisher_obj = graph.intern("PublisherCU");
        let publisher_event = graph.intern("OnPostDocument");
        let file = graph.get_shared_path(Path::new("subscriber.al"));

        graph.add_event_subscription(EventSubscription {
            subscriber: QualifiedName {
                object: subscriber_obj,
                procedure: subscriber_proc,
            },
            file: file.clone(),
            range: make_range(10, 20),
            publisher_object_type: Some(ObjectType::Codeunit),
            publisher_object: publisher_obj,
            publisher_event,
        });

        let publisher_qname = QualifiedName {
            object: publisher_obj,
            procedure: publisher_event,
        };
        assert_eq!(graph.get_event_subscribers(&publisher_qname).len(), 1);

        graph.remove_file(Path::new("subscriber.al"));
        assert!(graph.get_event_subscribers(&publisher_qname).is_empty());
    }

    // ==================== Utility Method Tests ====================

    #[test]
    fn test_call_site_count() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCU");
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
            complexity: 0,
            parameter_count: 0,
        });
        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: callee,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
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
                caller,
                callee_object: None,
                callee_method: callee,
            },
        );

        assert_eq!(graph.call_site_count(), 1);

        // After removing file, tombstoned call sites shouldn't count
        graph.remove_file(Path::new("test.al"));
        assert_eq!(graph.call_site_count(), 0);
    }

    #[test]
    fn test_get_definitions_in_file() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCU");
        let proc1 = graph.intern("Proc1");
        let proc2 = graph.intern("Proc2");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: proc1,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });
        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: proc2,
            kind: DefinitionKind::Trigger,
            complexity: 0,
            parameter_count: 0,
        });

        let defs = graph.get_definitions_in_file(Path::new("test.al"));
        assert_eq!(defs.len(), 2);
    }

    #[test]
    fn test_get_definitions_in_file_unknown() {
        let graph = CallGraph::new();
        let defs = graph.get_definitions_in_file(Path::new("unknown.al"));
        assert!(defs.is_empty());
    }

    #[test]
    fn test_get_incoming_call_count_with_events() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCU");
        let proc = graph.intern("OnPost");
        let caller = graph.intern("Caller");
        let sub_obj = graph.intern("SubCU");
        let sub_proc = graph.intern("HandlePost");
        let file = graph.get_shared_path(Path::new("test.al"));
        let sub_file = graph.get_shared_path(Path::new("sub.al"));

        graph.register_object(obj, ObjectType::Codeunit);

        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });
        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: caller,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        let proc_qname = QualifiedName {
            object: obj,
            procedure: proc,
        };
        let caller_qname = QualifiedName {
            object: obj,
            procedure: caller,
        };

        // Add a direct call
        graph.add_call_site(
            caller_qname,
            CallSite {
                file: file.clone(),
                range: make_range(30, 30),
                caller,
                callee_object: None,
                callee_method: proc,
            },
        );

        // Add an event subscription
        graph.add_event_subscription(EventSubscription {
            subscriber: QualifiedName {
                object: sub_obj,
                procedure: sub_proc,
            },
            file: sub_file,
            range: make_range(10, 20),
            publisher_object_type: Some(ObjectType::Codeunit),
            publisher_object: obj,
            publisher_event: proc,
        });

        // Should count both direct call and event subscriber
        assert_eq!(graph.get_incoming_call_count(&proc_qname), 2);
    }

    #[test]
    fn test_get_unused_procedures() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCU");
        let used_proc = graph.intern("UsedProc");
        let unused_proc = graph.intern("UnusedProc");
        let trigger = graph.intern("OnRun");
        let caller = graph.intern("Caller");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.register_object(obj, ObjectType::Codeunit);

        // Add procedures and a trigger
        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: used_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });
        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(25, 35),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: unused_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });
        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(40, 50),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: trigger,
            kind: DefinitionKind::Trigger,
            complexity: 0,
            parameter_count: 0,
        });
        graph.add_definition(Definition {
            file: file.clone(),
            range: make_range(55, 65),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: caller,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        // Caller calls UsedProc
        let caller_qname = QualifiedName {
            object: obj,
            procedure: caller,
        };
        graph.add_call_site(
            caller_qname,
            CallSite {
                file,
                range: make_range(60, 60),
                caller,
                callee_object: None,
                callee_method: used_proc,
            },
        );

        let unused = graph.get_unused_procedures();
        // unused_proc and caller should be unused (caller calls used_proc but nobody calls caller)
        // trigger should NOT appear (triggers are excluded)
        assert!(unused.iter().any(|(q, _)| q.procedure == unused_proc));
        assert!(unused.iter().any(|(q, _)| q.procedure == caller));
        assert!(!unused.iter().any(|(q, _)| q.procedure == trigger));
    }

    #[test]
    fn test_iter_definitions() {
        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCU");
        let proc = graph.intern("Proc1");
        let file = graph.get_shared_path(Path::new("test.al"));

        graph.add_definition(Definition {
            file,
            range: make_range(10, 20),
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        assert_eq!(graph.iter_definitions().count(), 1);
    }

    #[test]
    fn test_default_trait() {
        let graph = CallGraph::default();
        assert_eq!(graph.definition_count(), 0);
        assert_eq!(graph.call_site_count(), 0);
        assert_eq!(graph.external_definition_count(), 0);
    }

    // ==================== External Object Resolution Tests ====================

    #[test]
    fn test_resolve_call_via_external_object() {
        let mut graph = CallGraph::new();
        let caller_obj = graph.intern("LocalCU");
        let ext_obj = graph.intern("ExternalCU");
        let caller_proc = graph.intern("CallerProc");
        let ext_proc = graph.intern("ExtProc");
        let app_name = graph.intern("TestApp");
        let app_version = graph.intern("1.0.0");
        let caller_file = graph.get_shared_path(Path::new("local.al"));

        graph.register_object(caller_obj, ObjectType::Codeunit);
        graph.register_external_object(ext_obj, ObjectType::Codeunit);

        graph.add_definition(Definition {
            file: caller_file.clone(),
            range: make_range(10, 30),
            object_type: ObjectType::Codeunit,
            object_name: caller_obj,
            name: caller_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_external_definition(ExternalDefinition {
            source: ExternalSource {
                app_name,
                app_version,
            },
            object_type: ObjectType::Codeunit,
            object_name: ext_obj,
            name: ext_proc,
            kind: DefinitionKind::Procedure,
        });

        let caller_qname = QualifiedName {
            object: caller_obj,
            procedure: caller_proc,
        };

        // Call to ExternalCU.ExtProc - should resolve via external object
        graph.add_call_site(
            caller_qname,
            CallSite {
                file: caller_file,
                range: make_range(20, 20),
                caller: caller_proc,
                callee_object: Some(ext_obj),
                callee_method: ext_proc,
            },
        );

        // The outgoing calls should resolve to the external object
        let outgoing = graph.get_outgoing_calls(&caller_qname);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].callee_object, Some(ext_obj));
    }
}
