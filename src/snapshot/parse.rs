//! Deep-parse every source-bearing [`AppUnit`] in a snapshot into the owned IR.
//!
//! Symbol-only units (no embedded source) contribute no [`ParsedUnit`]; their
//! ABI is consumed by later resolution phases.

use rayon::prelude::*;

use crate::snapshot::identity::{AppId, Provenance};
use crate::snapshot::snapshot::AppSetSnapshot;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One parsed AL source file within a snapshot unit.
pub struct ParsedFile {
    pub virtual_path: String,
    pub file: al_syntax::ir::AlFile,
    pub provenance: Provenance,
}

/// All parsed files for one source-bearing app.
pub struct ParsedUnit {
    pub app: AppId,
    pub files: Vec<ParsedFile>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Parse every source file of every source-bearing app in `snap` in parallel.
///
/// Units whose `source` is `None` (symbol-only boundary apps) are skipped;
/// their ABI is used for resolution in later phases.
///
/// A local rayon thread pool is built with an explicit 32 MB stack per worker.
/// The `al_syntax` lowerer recurses into nested AL statements; the rayon
/// global-pool default is too shallow on Windows for large BC app packages.
pub fn parse_snapshot(snap: &AppSetSnapshot) -> Vec<ParsedUnit> {
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(32 * 1024 * 1024)
        .build()
        .expect("rayon pool for snapshot parse");
    pool.install(|| {
        snap.apps
            .iter()
            .filter_map(|unit| {
                let source = unit.source.as_ref()?;
                let files: Vec<ParsedFile> = source
                    .files
                    .par_iter()
                    .map(|f| ParsedFile {
                        virtual_path: f.virtual_path.clone(),
                        file: al_syntax::parse(&f.text),
                        provenance: unit.provenance.clone(),
                    })
                    .collect();
                Some(ParsedUnit {
                    app: unit.id.clone(),
                    files,
                })
            })
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_source_units_zero_panics() {
        let Some(ws) = std::env::var_os("CDO_WS")
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
        else {
            return;
        };
        let snap = crate::snapshot::SnapshotBuilder {
            workspace_root: ws,
            local_providers: vec![],
        }
        .build()
        .unwrap();
        let parsed = parse_snapshot(&snap);
        assert!(!parsed.is_empty());
        let total_files: usize = parsed.iter().map(|u| u.files.len()).sum();
        assert!(
            total_files > 1000,
            "deep parse should cover many files, got {total_files}"
        );
    }
}
