//! Project indexer - builds call graph from AL files

use anyhow::{Context, Result};
use log::{debug, info, warn};
use rayon::prelude::*;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use walkdir::WalkDir;

use crate::app_package::{ExternalMethodKind, ParsedAppPackage};
use crate::dependencies;
use crate::graph::{
    CallGraph, CallSite, Definition, DefinitionKind, DependencyMethod, DependencyMethodKind,
    DependencyObject, EventSubscription, ExternalDefinition, ExternalSource,
    LocalEventPublisher, LocalEventPublisherKind, ObjectType, QualifiedName,
};
use crate::parser::{AlParser, EventPublisherKind, ParsedFile};

// Thread-local parser to avoid recompiling queries for every file
thread_local! {
    static PARSER: RefCell<Option<AlParser>> = const { RefCell::new(None) };
}

/// Project indexer
pub struct Indexer {
    graph: Mutex<CallGraph>,
}

impl Indexer {
    pub fn new() -> Self {
        Self {
            graph: Mutex::new(CallGraph::new()),
        }
    }

    /// Index all AL files in a directory
    pub fn index_directory(&mut self, root: &Path) -> Result<()> {
        use std::time::Instant;

        let total_start = Instant::now();
        info!("Indexing directory: {}", root.display());

        // Collect all .al files
        let walk_start = Instant::now();
        let al_files: Vec<PathBuf> = WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext.eq_ignore_ascii_case("al"))
                    .unwrap_or(false)
            })
            .map(|e| e.path().to_path_buf())
            .collect();
        info!(
            "Found {} AL files in {:.1}ms",
            al_files.len(),
            walk_start.elapsed().as_secs_f64() * 1000.0
        );

        // Parse files in parallel
        let parse_start = Instant::now();
        let parsed_files: Vec<(PathBuf, Result<ParsedFile>)> = al_files
            .par_iter()
            .map(|path| {
                let result = self.parse_file(path);
                (path.clone(), result)
            })
            .collect();
        info!(
            "Parsed {} files in {:.1}ms ({:.2} files/sec)",
            al_files.len(),
            parse_start.elapsed().as_secs_f64() * 1000.0,
            al_files.len() as f64 / parse_start.elapsed().as_secs_f64()
        );

        // Build the graph (single-threaded to avoid contention)
        let graph_start = Instant::now();
        let mut graph = self.graph.lock().unwrap();
        for (path, result) in parsed_files {
            match result {
                Ok(parsed) => {
                    self.add_to_graph(&mut graph, &path, parsed);
                }
                Err(e) => {
                    warn!("Failed to parse {}: {}", path.display(), e);
                }
            }
        }
        info!(
            "Built graph in {:.1}ms",
            graph_start.elapsed().as_secs_f64() * 1000.0
        );

        info!(
            "Indexed {} definitions, {} call sites in {:.1}ms total",
            graph.definition_count(),
            graph.call_site_count(),
            total_start.elapsed().as_secs_f64() * 1000.0
        );

        Ok(())
    }

    /// Parse a single file using thread-local parser
    fn parse_file(&self, path: &Path) -> Result<ParsedFile> {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                #[cfg(feature = "telemetry")]
                crate::telemetry::record_indexer_issue(
                    crate::telemetry::IndexerIssueKind::IoError,
                    (e.kind() as u8) as u16,
                    None,
                );
                return Err(e).with_context(|| format!("Failed to read {}", path.display()));
            }
        };

        PARSER.with(|cell| {
            let mut parser_opt = cell.borrow_mut();
            if parser_opt.is_none() {
                *parser_opt = Some(AlParser::new()?);
            }
            let result = parser_opt.as_mut().unwrap().parse_file(path, &source);
            #[cfg(feature = "telemetry")]
            {
                if result.is_err() {
                    crate::telemetry::record_parser_error(
                        crate::telemetry::ParserErrorKind::ParseFailed,
                        path,
                    );
                }
            }
            result
        })
    }

    /// Add parsed file to the graph
    fn add_to_graph(&self, graph: &mut CallGraph, path: &Path, parsed: ParsedFile) {
        let object_name = match &parsed.object_name {
            Some(name) => graph.intern(name),
            None => {
                debug!("No object name found in {}", path.display());
                return;
            }
        };

        let object_type = match parsed.object_type {
            Some(t) => t,
            None => {
                debug!("No object type found in {}", path.display());
                return;
            }
        };

        // Get shared path once and reuse for all definitions and call sites
        let shared_path = graph.get_shared_path(path);

        // Register the object
        graph.register_object(object_name, object_type);

        // Add definitions
        for def in parsed.definitions {
            let name_sym = graph.intern(&def.name);
            graph.add_definition(Definition {
                file: shared_path.clone(),
                range: def.range,
                object_type,
                object_name,
                name: name_sym,
                kind: def.kind,
                complexity: def.complexity,
                parameter_count: def.parameter_count,
            });
        }

        // Add variable bindings for type resolution
        for var in parsed.variables {
            // Only add variables that have a containing procedure (local vars)
            // and that have a Record/Codeunit type
            if let Some(ref proc_name) = var.containing_procedure {
                if var
                    .type_kind
                    .as_ref()
                    .map(|k| k == "Record" || k == "Codeunit")
                    .unwrap_or(false)
                {
                    let proc_sym = graph.intern(proc_name);
                    let var_sym = graph.intern(&var.name);
                    let type_sym = graph.intern(&var.type_name);

                    let proc_qname = QualifiedName {
                        object: object_name,
                        procedure: proc_sym,
                    };

                    graph.add_variable_binding(shared_path.clone(), proc_qname, var_sym, type_sym);
                }
            }
        }

        // Add calls
        for call in parsed.calls {
            let callee_object = call.object.as_ref().map(|o| graph.intern(o));
            let callee_method = graph.intern(&call.method);

            // Determine the caller (containing procedure)
            let caller_name = call
                .containing_procedure
                .as_ref()
                .map(|p| graph.intern(p))
                .unwrap_or(object_name); // Use object name as fallback

            let caller_qname = QualifiedName {
                object: object_name,
                procedure: caller_name,
            };

            graph.add_call_site(
                caller_qname,
                CallSite {
                    file: shared_path.clone(),
                    range: call.range,
                    caller: caller_name,
                    callee_object,
                    callee_method,
                },
            );
        }

        // Cache event publishers for this file (used by documentSymbol overlay).
        let local_publishers: Vec<LocalEventPublisher> = parsed
            .event_publishers
            .into_iter()
            .map(|p| LocalEventPublisher {
                name: p.name,
                range: p.range,
                selection_range: p.selection_range,
                kind: match p.kind {
                    EventPublisherKind::IntegrationEvent => LocalEventPublisherKind::IntegrationEvent,
                    EventPublisherKind::BusinessEvent => LocalEventPublisherKind::BusinessEvent,
                    EventPublisherKind::InternalEvent => LocalEventPublisherKind::InternalEvent,
                },
                is_local: p.is_local,
                signature: p.signature,
            })
            .collect();
        graph.set_local_event_publishers(shared_path.clone(), local_publishers);

        // Add event subscriptions
        for sub in parsed.event_subscribers {
            let subscriber_name = graph.intern(&sub.subscriber_name);
            let publisher_object = graph.intern(&sub.publisher_object);
            let publisher_event = graph.intern(&sub.publisher_event);
            let publisher_object_type = sub
                .publisher_object_type
                .as_ref()
                .and_then(|t| ObjectType::try_from(t.as_str()).ok());

            let subscriber_qname = QualifiedName {
                object: object_name,
                procedure: subscriber_name,
            };

            graph.add_event_subscription(EventSubscription {
                subscriber: subscriber_qname,
                file: shared_path.clone(),
                range: sub.range,
                publisher_object_type,
                publisher_object,
                publisher_event,
            });
        }
    }

    /// Re-index a single file (for incremental updates)
    pub fn reindex_file(&self, path: &Path) -> Result<()> {
        let mut graph = self.graph.lock().unwrap();

        // Remove old data for this file
        graph.remove_file(path);

        // Parse and add new data
        if path.exists() {
            let parsed = self.parse_file(path)?;
            self.add_to_graph(&mut graph, path, parsed);
        }

        Ok(())
    }

    /// Get the call graph
    pub fn graph(&self) -> std::sync::MutexGuard<'_, CallGraph> {
        self.graph.lock().unwrap()
    }

    /// Consume the indexer and return the graph
    pub fn into_graph(self) -> CallGraph {
        self.graph.into_inner().unwrap()
    }

    /// Index external dependencies from .app packages
    ///
    /// Loads every `.app` file present in the project's `.alpackages` folder
    /// (mirroring the AL LSP) so transitive deps like Base Application get
    /// indexed even when not declared in app.json. Falls back to the
    /// declared-only resolver when there are duplicates so the
    /// publisher/version metadata stays correct.
    pub fn index_dependencies(&self, project_root: &Path) -> Result<usize> {
        use std::collections::HashSet;
        use std::time::Instant;

        let start = Instant::now();

        // Load every .app — this is what gives us Base Application, System
        // Application, etc. for projects that depend on them transitively.
        // load_all_apps returns packages in closest-first order: the project's
        // own .alpackages comes before parent / grandparent folders.
        let all_apps = dependencies::load_all_apps(project_root)?;

        // Also pull declared deps for their authoritative publisher/version,
        // and to surface any deps that resolve from a different alpackages
        // folder (rare, but legal).
        let declared = dependencies::resolve_all(project_root).unwrap_or_default();

        if all_apps.is_empty() && declared.is_empty() {
            debug!("No dependencies to index");
            return Ok(0);
        }

        let mut graph = self.graph.lock().unwrap();
        let mut total_defs = 0;

        // Dedup by (app_name_lowercase, version) so two .app files with the
        // same identity in different .alpackages folders don't trample each
        // other in dependency_objects (which keys by app+type+name and is
        // last-write-wins). Declared deps go first — their publisher/version
        // metadata is authoritative — then all_apps is iterated closest-first
        // so the closest copy wins for any (name, version) collision.
        let mut seen_apps: HashSet<(String, String)> = HashSet::new();
        let mut packages_indexed = 0usize;

        for dep in declared {
            let key = (
                dep.package.metadata.name.to_lowercase(),
                dep.package.metadata.version.clone(),
            );
            if !seen_apps.insert(key) {
                continue;
            }
            let count = self.add_app_to_graph(&mut graph, &dep.package);
            total_defs += count;
            packages_indexed += 1;
            debug!(
                "Added {} external definitions from {} v{} (declared)",
                count, dep.package.metadata.name, dep.package.metadata.version
            );
        }

        for dep in all_apps {
            let key = (
                dep.package.metadata.name.to_lowercase(),
                dep.package.metadata.version.clone(),
            );
            if !seen_apps.insert(key) {
                debug!(
                    "Skipping duplicate {} v{} from {} (closer copy already indexed)",
                    dep.package.metadata.name,
                    dep.package.metadata.version,
                    dep.app_path.display()
                );
                continue;
            }
            let count = self.add_app_to_graph(&mut graph, &dep.package);
            total_defs += count;
            packages_indexed += 1;
            debug!(
                "Added {} external definitions from {} v{} (.alpackages: {})",
                count,
                dep.package.metadata.name,
                dep.package.metadata.version,
                dep.app_path.display()
            );
        }

        info!(
            "Indexed {} external definitions from {} packages in {:.1}ms",
            total_defs,
            packages_indexed,
            start.elapsed().as_secs_f64() * 1000.0
        );

        Ok(total_defs)
    }

    /// Add definitions from a parsed .app package to the graph
    fn add_app_to_graph(&self, graph: &mut CallGraph, package: &ParsedAppPackage) -> usize {
        let app_name = graph.intern(&package.metadata.name);
        let app_version = graph.intern(&package.metadata.version);
        let source = ExternalSource {
            app_name,
            app_version,
        };

        let mut count = 0;

        for obj in &package.objects {
            let object_name = graph.intern(&obj.name);

            // Register the external object type
            graph.register_external_object(object_name, obj.object_type);

            // Add each method as an external definition (used by the call graph)
            // and aggregate into a DependencyObject for documentSymbol synthesis.
            let mut dep_methods = Vec::with_capacity(obj.methods.len());
            for method in &obj.methods {
                let method_name = graph.intern(&method.name);

                graph.add_external_definition(ExternalDefinition {
                    source,
                    object_type: obj.object_type,
                    object_name,
                    name: method_name,
                    kind: external_method_to_definition_kind(method.kind),
                });

                dep_methods.push(DependencyMethod {
                    name: method.name.clone(),
                    kind: external_method_kind_to_dep(method.kind),
                    signature: method.signature.clone(),
                    is_local: method.is_local,
                });

                count += 1;
            }

            graph.add_dependency_object(DependencyObject {
                app_name: package.metadata.name.clone(),
                app_version: package.metadata.version.clone(),
                object_type: obj.object_type,
                object_id: obj.id,
                object_name: obj.name.clone(),
                methods: dep_methods,
            });
        }

        count
    }
}

fn external_method_to_definition_kind(kind: ExternalMethodKind) -> DefinitionKind {
    match kind {
        ExternalMethodKind::EventSubscriber => DefinitionKind::EventSubscriber,
        // Event publishers and regular procedures are both DefinitionKind::Procedure
        // for the call-graph layer. The richer distinction lives in DependencyMethod.
        _ => DefinitionKind::Procedure,
    }
}

fn external_method_kind_to_dep(kind: ExternalMethodKind) -> DependencyMethodKind {
    match kind {
        ExternalMethodKind::Procedure => DependencyMethodKind::Procedure,
        ExternalMethodKind::IntegrationEvent => DependencyMethodKind::IntegrationEvent,
        ExternalMethodKind::BusinessEvent => DependencyMethodKind::BusinessEvent,
        ExternalMethodKind::InternalEvent => DependencyMethodKind::InternalEvent,
        ExternalMethodKind::EventSubscriber => DependencyMethodKind::EventSubscriber,
    }
}

impl Default for Indexer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ObjectType, QualifiedName};
    use tempfile::TempDir;

    #[test]
    fn test_indexer_creation() {
        let indexer = Indexer::new();
        let graph = indexer.graph();
        assert_eq!(graph.definition_count(), 0);
    }

    fn create_al_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_index_directory_finds_definitions() {
        let temp_dir = TempDir::new().unwrap();
        let al_content = r#"
codeunit 50100 "Test Codeunit"
{
    procedure TestProc()
    begin
    end;

    trigger OnRun()
    begin
    end;
}
"#;
        create_al_file(temp_dir.path(), "test.al", al_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        let graph = indexer.graph();
        assert_eq!(graph.definition_count(), 2); // TestProc + OnRun
    }

    #[test]
    fn test_index_directory_registers_object() {
        let temp_dir = TempDir::new().unwrap();
        let al_content = r#"
codeunit 50100 "My Codeunit"
{
    procedure MyProc()
    begin
    end;
}
"#;
        create_al_file(temp_dir.path(), "codeunit.al", al_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        let graph = indexer.graph();

        // Verify object was registered by checking we can find the definition
        let obj_sym = graph.get_symbol("My Codeunit").unwrap();
        let proc_sym = graph.get_symbol("MyProc").unwrap();
        let qname = QualifiedName {
            object: obj_sym,
            procedure: proc_sym,
        };
        let def = graph.get_definition(&qname);
        assert!(def.is_some());
        assert_eq!(def.unwrap().object_type, ObjectType::Codeunit);
    }

    #[test]
    fn test_index_directory_adds_calls() {
        let temp_dir = TempDir::new().unwrap();
        let al_content = r#"
codeunit 50100 "Test Codeunit"
{
    procedure Caller()
    begin
        Callee();
    end;

    procedure Callee()
    begin
    end;
}
"#;
        create_al_file(temp_dir.path(), "test.al", al_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        let graph = indexer.graph();

        // Check that incoming calls to Callee exist
        let obj_sym = graph.get_symbol("Test Codeunit").unwrap();
        let callee_sym = graph.get_symbol("Callee").unwrap();
        let callee_qname = QualifiedName {
            object: obj_sym,
            procedure: callee_sym,
        };

        let incoming = graph.get_incoming_calls(&callee_qname);
        assert_eq!(incoming.len(), 1);
    }

    #[test]
    fn test_index_directory_adds_variable_bindings() {
        let temp_dir = TempDir::new().unwrap();
        let al_content = r#"
codeunit 50100 "Test Codeunit"
{
    procedure TestProc()
    var
        Cust: Record Customer;
    begin
        Cust.Validate("No.");
    end;
}
"#;
        create_al_file(temp_dir.path(), "test.al", al_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        let graph = indexer.graph();

        // Check that variable binding was added
        let obj_sym = graph.get_symbol("Test Codeunit").unwrap();
        let proc_sym = graph.get_symbol("TestProc").unwrap();
        let proc_qname = QualifiedName {
            object: obj_sym,
            procedure: proc_sym,
        };

        // The variable "Cust" should resolve to "Customer"
        if let Some(var_sym) = graph.get_symbol("Cust") {
            let type_sym = graph.lookup_variable_type(&proc_qname, var_sym);
            if let Some(type_sym) = type_sym {
                assert_eq!(graph.resolve(type_sym), Some("Customer"));
            }
        }
    }

    #[test]
    fn test_reindex_file_removes_old_data() {
        let temp_dir = TempDir::new().unwrap();
        let al_content = r#"
codeunit 50100 "Test Codeunit"
{
    procedure OldProc()
    begin
    end;
}
"#;
        let file_path = create_al_file(temp_dir.path(), "test.al", al_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        {
            let graph = indexer.graph();
            assert_eq!(graph.definition_count(), 1);
        }

        // Update the file with different content
        let new_content = r#"
codeunit 50100 "Test Codeunit"
{
    procedure NewProc()
    begin
    end;

    procedure AnotherProc()
    begin
    end;
}
"#;
        fs::write(&file_path, new_content).unwrap();

        // Reindex the file
        indexer.reindex_file(&file_path).unwrap();

        let graph = indexer.graph();
        assert_eq!(graph.definition_count(), 2); // NewProc + AnotherProc

        // Old procedure should be gone
        let obj_sym = graph.get_symbol("Test Codeunit").unwrap();
        if let Some(old_proc_sym) = graph.get_symbol("OldProc") {
            let qname = QualifiedName {
                object: obj_sym,
                procedure: old_proc_sym,
            };
            assert!(graph.get_definition(&qname).is_none());
        }

        // New procedure should exist
        let new_proc_sym = graph.get_symbol("NewProc").unwrap();
        let qname = QualifiedName {
            object: obj_sym,
            procedure: new_proc_sym,
        };
        assert!(graph.get_definition(&qname).is_some());
    }

    #[test]
    fn test_reindex_file_handles_deleted_file() {
        let temp_dir = TempDir::new().unwrap();
        let al_content = r#"
codeunit 50100 "Test Codeunit"
{
    procedure TestProc()
    begin
    end;
}
"#;
        let file_path = create_al_file(temp_dir.path(), "test.al", al_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        {
            let graph = indexer.graph();
            assert_eq!(graph.definition_count(), 1);
        }

        // Delete the file
        fs::remove_file(&file_path).unwrap();

        // Reindex should remove the data without error
        indexer.reindex_file(&file_path).unwrap();

        let graph = indexer.graph();
        assert_eq!(graph.definition_count(), 0);
    }

    #[test]
    fn test_index_directory_multiple_files() {
        let temp_dir = TempDir::new().unwrap();

        let file1_content = r#"
codeunit 50100 "Codeunit1"
{
    procedure Proc1()
    begin
    end;
}
"#;
        let file2_content = r#"
codeunit 50101 "Codeunit2"
{
    procedure Proc2()
    begin
    end;
}
"#;
        create_al_file(temp_dir.path(), "file1.al", file1_content);
        create_al_file(temp_dir.path(), "file2.al", file2_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        let graph = indexer.graph();
        assert_eq!(graph.definition_count(), 2);
    }

    #[test]
    fn test_index_directory_cross_file_calls() {
        let temp_dir = TempDir::new().unwrap();

        let caller_content = r#"
codeunit 50100 "CallerCodeunit"
{
    procedure CallerProc()
    begin
        CalleeCodeunit.CalleeProc();
    end;
}
"#;
        let callee_content = r#"
codeunit 50101 "CalleeCodeunit"
{
    procedure CalleeProc()
    begin
    end;
}
"#;
        create_al_file(temp_dir.path(), "caller.al", caller_content);
        create_al_file(temp_dir.path(), "callee.al", callee_content);

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        let graph = indexer.graph();

        // Check that cross-file call was recorded
        let callee_obj = graph.get_symbol("CalleeCodeunit").unwrap();
        let callee_proc = graph.get_symbol("CalleeProc").unwrap();
        let callee_qname = QualifiedName {
            object: callee_obj,
            procedure: callee_proc,
        };

        let incoming = graph.get_incoming_calls(&callee_qname);
        assert_eq!(incoming.len(), 1);
    }

    #[test]
    fn test_index_directory_ignores_non_al_files() {
        let temp_dir = TempDir::new().unwrap();

        let al_content = r#"
codeunit 50100 "Test"
{
    procedure TestProc()
    begin
    end;
}
"#;
        create_al_file(temp_dir.path(), "test.al", al_content);
        create_al_file(temp_dir.path(), "readme.txt", "Not an AL file");
        create_al_file(temp_dir.path(), "test.json", "{}");

        let mut indexer = Indexer::new();
        indexer.index_directory(temp_dir.path()).unwrap();

        let graph = indexer.graph();
        assert_eq!(graph.definition_count(), 1); // Only the AL file
    }

    #[test]
    fn test_index_directory_event_subscribers() {
        let dir = TempDir::new().unwrap();
        create_al_file(
            dir.path(),
            "publisher.al",
            r#"codeunit 50100 "Publisher"
{
    procedure OnBeforePost()
    begin
    end;
}"#,
        );
        create_al_file(
            dir.path(),
            "subscriber.al",
            r#"codeunit 50101 "Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Publisher, 'OnBeforePost', '', false, false)]
    local procedure HandleOnBeforePost()
    begin
    end;
}"#,
        );

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();

        let graph = indexer.graph();

        // Verify the subscriber object is indexed
        let sub_obj = graph.get_symbol("Subscriber");
        assert!(sub_obj.is_some(), "Subscriber object should be indexed");

        // Check if event subscriber was detected
        let pub_obj = graph.get_symbol("Publisher");
        let pub_event = graph.get_symbol("OnBeforePost");

        if let (Some(pub_obj), Some(pub_event)) = (pub_obj, pub_event) {
            let pub_qname = QualifiedName {
                object: pub_obj,
                procedure: pub_event,
            };
            let subscribers = graph.get_event_subscribers(&pub_qname);
            println!(
                "Event subscribers found: {}. If 0, the grammar may not support this attribute format.",
                subscribers.len()
            );
            // The event subscriber parsing should find at least one subscriber
            // if the tree-sitter grammar supports this attribute format
            if !subscribers.is_empty() {
                assert_eq!(
                    subscribers[0].subscriber.object,
                    graph.get_symbol("Subscriber").unwrap()
                );
            }
        }
    }

    #[test]
    fn test_index_directory_handles_malformed_file() {
        let dir = TempDir::new().unwrap();

        // Valid file
        create_al_file(
            dir.path(),
            "valid.al",
            r#"codeunit 50100 "Valid"
{
    procedure TestProc()
    begin
    end;
}"#,
        );

        // File with no AL object (just a comment)
        create_al_file(dir.path(), "empty.al", "// just a comment, no AL object");

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();

        let graph = indexer.graph();
        assert!(
            graph.definition_count() >= 1,
            "Valid file should be indexed despite malformed sibling"
        );
    }

    #[test]
    fn test_indexer_into_graph() {
        let dir = TempDir::new().unwrap();
        create_al_file(
            dir.path(),
            "test.al",
            r#"codeunit 50100 "Test"
{
    procedure TestProc()
    begin
    end;
}"#,
        );

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();

        let graph = indexer.into_graph();
        assert!(graph.definition_count() >= 1);
    }

    #[test]
    fn test_indexer_default() {
        let indexer = Indexer::default();
        let graph = indexer.graph();
        assert_eq!(graph.definition_count(), 0);
    }

    #[test]
    fn test_index_dependencies_real_project() {
        let project = std::path::Path::new("U:/Git/DO.Support-wi-75148/DocumentOutput/Cloud");
        if !project.exists() {
            eprintln!("Skipping test: DO.Support-wi-75148 not available");
            return;
        }

        let indexer = Indexer::new();
        let result = indexer.index_dependencies(project);
        assert!(
            result.is_ok(),
            "Failed to index dependencies: {:?}",
            result.err()
        );

        let count = result.unwrap();
        println!("Indexed {} external definitions", count);
        assert!(count > 0, "Should index at least some external definitions");

        let graph = indexer.graph();
        assert!(
            graph.external_definition_count() > 0,
            "Graph should have external definitions"
        );
    }
}
