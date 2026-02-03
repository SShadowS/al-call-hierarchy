use anyhow::Result;
use clap::{Parser, ValueEnum};
use log::info;
use std::path::PathBuf;

mod analysis;
mod app_package;
mod dependencies;
mod graph;
mod handlers;
mod indexer;
mod language;
mod parser;
mod protocol;
mod server;
mod watcher;

use indexer::Indexer;
use server::run_server;

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
    Csv,
}

#[derive(Parser, Debug)]
#[command(name = "al-call-hierarchy")]
#[command(about = "Blazing-fast call hierarchy server for AL (Business Central)")]
struct Args {
    /// Path to the AL project root (CLI mode - index and report stats)
    #[arg(short, long)]
    project: Option<PathBuf>,

    /// Run in LSP server mode (stdio). This is the default if --project is not specified.
    #[arg(long)]
    lsp: bool,

    /// Run code quality analysis (requires --project)
    #[arg(short, long)]
    analyze: bool,

    /// Output format for analysis results
    #[arg(short, long, value_enum, default_value = "text")]
    format: OutputFormat,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging - suppress for JSON output
    let log_level = if matches!(args.format, OutputFormat::Json) && args.analyze {
        log::LevelFilter::Off
    } else if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    env_logger::Builder::new()
        .filter_level(log_level)
        .init();

    if let Some(project) = args.project {
        if args.analyze {
            // Analysis mode
            run_analysis(&project, &args.format)?;
        } else {
            // CLI mode for testing/indexing
            info!("Indexing project: {}", project.display());
            let mut indexer = Indexer::new();
            indexer.index_directory(&project)?;

            // Index external dependencies from .app packages
            if project.join("app.json").exists() {
                if let Err(e) = indexer.index_dependencies(&project) {
                    log::warn!("Failed to index dependencies: {}", e);
                }
            }

            let graph = indexer.into_graph();
            info!("Indexed {} definitions", graph.definition_count());
            info!("Indexed {} external definitions", graph.external_definition_count());
            info!("Found {} call sites", graph.call_site_count());
        }
    } else {
        // LSP server mode (default)
        info!("Starting AL Call Hierarchy LSP server");
        run_server()?;
    }

    Ok(())
}

/// Run code quality analysis on a project
fn run_analysis(project: &PathBuf, format: &OutputFormat) -> Result<()> {
    use analysis::{build_summary, generate_findings, AnalysisResult, ProcedureMetrics};
    use rayon::prelude::*;
    use std::cell::RefCell;
    use std::fs;
    use std::time::Instant;
    use tree_sitter::Parser as TsParser;
    use walkdir::WalkDir;

    let start = Instant::now();
    info!("Analyzing project: {}", project.display());

    // Collect all .al files
    let al_files: Vec<PathBuf> = WalkDir::new(project)
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

    info!("Found {} AL files", al_files.len());

    // Thread-local parser for parallel processing
    thread_local! {
        static PARSER: RefCell<Option<TsParser>> = const { RefCell::new(None) };
    }

    // Parse files and collect metrics in parallel
    let all_metrics: Vec<ProcedureMetrics> = al_files
        .par_iter()
        .flat_map(|path| {
            let source = match fs::read_to_string(path) {
                Ok(s) => s,
                Err(_) => return vec![],
            };

            PARSER.with(|cell| {
                let mut parser_opt = cell.borrow_mut();
                if parser_opt.is_none() {
                    let mut parser = TsParser::new();
                    if parser.set_language(&language::language()).is_err() {
                        return vec![];
                    }
                    *parser_opt = Some(parser);
                }

                let parser = parser_opt.as_mut().unwrap();
                let tree = match parser.parse(&source, None) {
                    Some(t) => t,
                    None => return vec![],
                };

                extract_metrics_from_tree(&tree.root_node(), &source, path)
            })
        })
        .collect();

    // Generate findings
    let mut all_findings = Vec::new();
    for metrics in &all_metrics {
        all_findings.extend(generate_findings(metrics));
    }

    // Build summary
    let summary = build_summary(&all_metrics, &all_findings);

    let result = AnalysisResult {
        metrics: all_metrics,
        findings: all_findings,
        summary,
    };

    info!(
        "Analyzed {} procedures in {:.1}ms",
        result.summary.total_procedures,
        start.elapsed().as_secs_f64() * 1000.0
    );

    // Output results
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Csv => {
            print_csv(&result);
        }
        OutputFormat::Text => {
            print_text(&result, project);
        }
    }

    Ok(())
}

/// Extract metrics from a parsed tree
fn extract_metrics_from_tree(
    root: &tree_sitter::Node,
    source: &str,
    path: &PathBuf,
) -> Vec<analysis::ProcedureMetrics> {
    let mut metrics = Vec::new();
    let mut object_type = String::new();
    let mut object_name = String::new();

    // Find object declaration and procedures
    let mut cursor = root.walk();
    extract_object_and_procedures(
        &mut cursor,
        source,
        path,
        &mut object_type,
        &mut object_name,
        &mut metrics,
    );

    metrics
}

fn extract_object_and_procedures(
    cursor: &mut tree_sitter::TreeCursor,
    source: &str,
    path: &PathBuf,
    object_type: &mut String,
    object_name: &mut String,
    metrics: &mut Vec<analysis::ProcedureMetrics>,
) {
    use analysis::{calculate_complexity, calculate_quality_score};

    let node = cursor.node();
    let kind = node.kind();

    // Detect object declarations (top-level AL object types only)
    // These are the main object declarations, not variable/parameter declarations
    let is_object_declaration = matches!(
        kind,
        "codeunit_declaration"
            | "table_declaration"
            | "page_declaration"
            | "report_declaration"
            | "query_declaration"
            | "xmlport_declaration"
            | "enum_declaration"
            | "interface_declaration"
            | "controladdin_declaration"
            | "pageextension_declaration"
            | "tableextension_declaration"
            | "enumextension_declaration"
            | "permissionset_declaration"
            | "permissionsetextension_declaration"
            | "preproc_split_codeunit_declaration"
    );

    if is_object_declaration {
        if let Some(name_node) = node.child_by_field_name("object_name") {
            *object_name = node_text(&name_node, source).trim_matches('"').to_string();
        }
        // Extract type from kind (e.g., "codeunit_declaration" -> "Codeunit")
        *object_type = kind
            .strip_suffix("_declaration")
            .unwrap_or(kind)
            .replace("preproc_split_", "")
            .split('_')
            .map(|s| {
                let mut c = s.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join("");
    }

    // Detect procedures and triggers
    if kind == "procedure" || kind == "trigger_declaration" || kind == "named_trigger" || kind == "onrun_trigger" {
        let proc_name = extract_procedure_name(&node, source);
        let complexity = calculate_complexity(&node);
        let line_count = (node.end_position().row - node.start_position().row + 1) as u32;
        let param_count = count_parameters(&node, source);
        let quality_score = calculate_quality_score(complexity, line_count, param_count);

        // Use relative path for cleaner output
        let file_str = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        metrics.push(analysis::ProcedureMetrics {
            object_type: object_type.clone(),
            object_name: object_name.clone(),
            procedure_name: proc_name,
            file: file_str,
            line: node.start_position().row as u32 + 1,
            complexity,
            line_count,
            parameter_count: param_count,
            quality_score,
        });
    }

    // Recurse into children
    if cursor.goto_first_child() {
        loop {
            extract_object_and_procedures(cursor, source, path, object_type, object_name, metrics);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn extract_procedure_name(node: &tree_sitter::Node, source: &str) -> String {
    // Try name field first
    if let Some(name_node) = node.child_by_field_name("name") {
        return node_text(&name_node, source).trim_matches('"').to_string();
    }

    // For named triggers, extract from first child
    if node.kind() == "named_trigger" || node.kind() == "onrun_trigger" {
        if let Some(child) = node.child(0) {
            let text = node_text(&child, source);
            return text.trim_matches('"').to_string();
        }
    }

    node.kind().to_string()
}

fn count_parameters(node: &tree_sitter::Node, _source: &str) -> u32 {
    // Find parameter_list child and count parameters
    let mut count = 0;
    let mut cursor = node.walk();

    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "parameter_list" {
                // Count parameter children
                let mut param_cursor = child.walk();
                if param_cursor.goto_first_child() {
                    loop {
                        if param_cursor.node().kind() == "parameter" {
                            count += 1;
                        }
                        if !param_cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
                break;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    count
}

fn node_text<'a>(node: &tree_sitter::Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Print results in CSV format
fn print_csv(result: &analysis::AnalysisResult) {
    println!("object_type,object_name,procedure_name,file,line,complexity,line_count,parameter_count,quality_score");
    for m in &result.metrics {
        println!(
            "{},{},{},{},{},{},{},{},{:.1}",
            m.object_type,
            m.object_name,
            m.procedure_name,
            m.file,
            m.line,
            m.complexity,
            m.line_count,
            m.parameter_count,
            m.quality_score
        );
    }
}

/// Print results in human-readable text format
fn print_text(result: &analysis::AnalysisResult, project: &PathBuf) {
    println!("\nCode Quality Analysis: {}\n", project.display());
    println!("═══════════════════════════════════════════════════════════════════════════════\n");

    // Sort by complexity (descending)
    let mut sorted_metrics = result.metrics.clone();
    sorted_metrics.sort_by(|a, b| b.complexity.cmp(&a.complexity));

    println!("PROCEDURES (sorted by complexity):\n");
    println!(
        "{:<40} {:>4} {:>6} {:>6} {:>8}",
        "Procedure", "CC", "Lines", "Params", "Score"
    );
    println!("{}", "-".repeat(70));

    for m in sorted_metrics.iter().take(20) {
        let name = format!("{}.{}", m.object_name, m.procedure_name);
        let name_truncated = if name.len() > 38 {
            format!("{}...", &name[..35])
        } else {
            name
        };

        let severity = if m.complexity >= 10 {
            " [CRITICAL]"
        } else if m.complexity >= 5 {
            " [WARNING]"
        } else {
            ""
        };

        println!(
            "{:<40} {:>4} {:>6} {:>6} {:>7.1}{}",
            name_truncated, m.complexity, m.line_count, m.parameter_count, m.quality_score, severity
        );
    }

    if sorted_metrics.len() > 20 {
        println!("  ... and {} more procedures", sorted_metrics.len() - 20);
    }

    // Findings
    if !result.findings.is_empty() {
        println!("\nFINDINGS:\n");
        for f in &result.findings {
            let severity_str = match f.severity.as_str() {
                "critical" => "[CRITICAL]",
                "warning" => "[WARNING]",
                _ => "[INFO]",
            };
            println!("  {} {} - {}", severity_str, f.location, f.description);
        }
    }

    // Summary
    println!("\nSUMMARY:\n");
    println!("  Total procedures:     {}", result.summary.total_procedures);
    println!("  Average complexity:   {:.1}", result.summary.avg_complexity);
    println!("  Average quality score: {:.1}", result.summary.avg_quality_score);
    println!("  Critical findings:    {}", result.summary.critical_findings);
    println!("  Warning findings:     {}", result.summary.warning_findings);
    println!();
}
