use anyhow::Result;
use clap::Parser;
use log::info;
use std::path::PathBuf;

mod app_package;
mod dependencies;
mod graph;
mod handlers;
mod indexer;
mod language;
mod parser;
mod protocol;
mod resolver;
mod server;
mod watcher;

use indexer::Indexer;
use server::run_server;

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

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    env_logger::Builder::new()
        .filter_level(if args.verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .init();

    if let Some(project) = args.project {
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
    } else {
        // LSP server mode (default)
        info!("Starting AL Call Hierarchy LSP server");
        run_server()?;
    }

    Ok(())
}
