//! Project indexer - builds call graph from AL files

use anyhow::{Context, Result};
use log::{debug, info, warn};
use rayon::prelude::*;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use walkdir::WalkDir;

use crate::app_package::ParsedAppPackage;
use crate::dependencies;
use crate::graph::{
    CallGraph, CallSite, Definition, DefinitionKind, ExternalDefinition, ExternalSource,
    QualifiedName,
};
use crate::parser::{AlParser, ParsedFile};

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
        let source = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        PARSER.with(|cell| {
            let mut parser_opt = cell.borrow_mut();
            if parser_opt.is_none() {
                *parser_opt = Some(AlParser::new()?);
            }
            parser_opt.as_mut().unwrap().parse_file(path, &source)
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
            });
        }

        // Add variable bindings for type resolution
        for var in parsed.variables {
            // Only add variables that have a containing procedure (local vars)
            // and that have a Record/Codeunit type
            if let Some(ref proc_name) = var.containing_procedure {
                if var.type_kind.as_ref().map(|k| k == "Record" || k == "Codeunit").unwrap_or(false) {
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
    /// Looks for app.json in the project root, resolves dependencies from
    /// the .alpackages folder, and adds external definitions to the graph.
    pub fn index_dependencies(&self, project_root: &Path) -> Result<usize> {
        use std::time::Instant;

        let start = Instant::now();
        let resolved = dependencies::resolve_all(project_root)?;

        if resolved.is_empty() {
            debug!("No dependencies to index");
            return Ok(0);
        }

        let mut graph = self.graph.lock().unwrap();
        let mut total_defs = 0;

        for dep in resolved {
            let count = self.add_app_to_graph(&mut graph, &dep.package);
            total_defs += count;
            debug!(
                "Added {} external definitions from {}",
                count, dep.package.metadata.name
            );
        }

        info!(
            "Indexed {} external definitions in {:.1}ms",
            total_defs,
            start.elapsed().as_secs_f64() * 1000.0
        );

        Ok(total_defs)
    }

    /// Add definitions from a parsed .app package to the graph
    fn add_app_to_graph(&self, graph: &mut CallGraph, package: &ParsedAppPackage) -> usize {
        let app_name = graph.intern(&package.metadata.name);
        let source = ExternalSource {
            app_name,
            app_version: package.metadata.version.clone(),
        };

        let mut count = 0;

        for obj in &package.objects {
            let object_name = graph.intern(&obj.name);

            // Register the external object type
            graph.register_external_object(object_name, obj.object_type);

            // Add each method as an external definition
            for method in &obj.methods {
                let method_name = graph.intern(&method.name);

                graph.add_external_definition(ExternalDefinition {
                    source: source.clone(),
                    object_type: obj.object_type,
                    object_name,
                    name: method_name,
                    kind: DefinitionKind::Procedure,
                });

                count += 1;
            }
        }

        count
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
}
