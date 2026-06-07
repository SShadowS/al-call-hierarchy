//! R2.5a `aldump --r2.5a-merged-index` CLI smoke — invokes the ACTUAL `aldump`
//! binary on a committed `.app` fixture dir and asserts stdout BYTE-matches the
//! golden. Locks the CLI wiring (flag parsing + the single trailing newline:
//! `serialize_projection` appends `\n`, the binary uses `print!` not `println!`,
//! so stdout must equal the golden byte-for-byte, no extra blank line).

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn aldump_r2_5a_merged_index_stdout_matches_golden() {
    // `CARGO_BIN_EXE_aldump` is set by cargo for integration tests of the bin.
    let bin = env!("CARGO_BIN_EXE_aldump");
    let root = repo_root();

    for (fixture, golden) in [
        ("core-symbol-only", "core-symbol-only.r2.5a.golden.json"),
        ("source-included", "source-included.r2.5a.golden.json"),
    ] {
        let fixture_dir = root.join("tests/r2-5a-fixtures").join(fixture);
        let golden_path = root.join("tests/r2-5a-goldens").join(golden);
        let golden = std::fs::read(&golden_path)
            .unwrap_or_else(|e| panic!("read golden {}: {e}", golden_path.display()));

        let out = Command::new(bin)
            .arg("--r2.5a-merged-index")
            .arg(&fixture_dir)
            .output()
            .unwrap_or_else(|e| panic!("spawn aldump: {e}"));
        assert!(
            out.status.success(),
            "[{fixture}] aldump exited non-zero: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            out.stdout,
            golden,
            "[{fixture}] aldump --r2.5a-merged-index stdout must byte-match the golden \
             (stderr: {})",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
