//! `cargo run -p xtask -- gen-syntax [--check]`
//!
//! Generates the al-syntax *raw vocabulary* from the pinned tree-sitter-al
//! `node-types.json`: `RawKind` (exhaustive over NAMED node kinds + `Error`) and
//! `FieldName` (exhaustive over field names), plus a `GRAMMAR_NODE_TYPES_HASH`.
//!
//! The output is checked in and hash-guarded (al-syntax `build.rs` asserts the
//! committed grammar matches the hash). `--check` regenerates into memory and
//! diffs against the committed files, failing if they differ (CI drift guard).
//!
//! Design notes (owned-syntax-IR spec §3.2, Phase 0):
//! - Only NAMED kinds become `RawKind` variants. The lowerer classifies named
//!   children only; anonymous punctuation never reaches `RawKind::from_raw`, so an
//!   unknown string there is a genuine grammar/binary mismatch → panic (loud).
//! - `from_raw` panics on unknown; a removed/renamed kind that the hand-written
//!   `NodeKind` mapping still references becomes a compile error (the loud path).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sha2::{Digest, Sha256};

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let mode = std::env::args().nth(1).unwrap_or_default();
    if mode != "gen-syntax" {
        eprintln!("usage: cargo run -p xtask -- gen-syntax [--check]");
        return ExitCode::FAILURE;
    }

    let ws = workspace_root();
    let node_types = ws.join("tree-sitter-al/src/node-types.json");
    let out_dir = ws.join("crates/al-syntax/src/raw/generated");

    let bytes = match std::fs::read(&node_types) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {}: {e}", node_types.display());
            return ExitCode::FAILURE;
        }
    };
    let hash = hex(&Sha256::digest(&bytes));
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("node-types.json parse");
    let entries = json.as_array().expect("node-types.json is an array");

    let model = Model::from_node_types(entries, &hash);
    let files: Vec<(String, String)> = vec![
        ("raw_kind.rs".to_string(), model.gen_raw_kind()),
        ("field.rs".to_string(), model.gen_field()),
        ("mod.rs".to_string(), model.gen_mod()),
        // Plain sidecar (no header) read by al-syntax build.rs to assert the
        // checked-in grammar matches what this vocabulary was generated from.
        ("node-types.sha256".to_string(), format!("{hash}\n")),
    ];

    if check {
        let mut drift = false;
        for (name, content) in &files {
            let path = out_dir.join(name);
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            if normalize(&on_disk) != normalize(content) {
                eprintln!("DRIFT: {} is stale — run `cargo run -p xtask -- gen-syntax`", path.display());
                drift = true;
            }
        }
        return if drift { ExitCode::FAILURE } else {
            println!("gen-syntax --check: generated files up to date ({} named kinds, {} fields, hash {})",
                model.kinds.len(), model.fields.len(), &hash[..12]);
            ExitCode::SUCCESS
        };
    }

    std::fs::create_dir_all(&out_dir).expect("create generated dir");
    for (name, content) in &files {
        let path = out_dir.join(name);
        std::fs::write(&path, content).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        println!("wrote {}", path.display());
    }
    println!(
        "gen-syntax: {} named kinds, {} fields, node-types.json hash {}",
        model.kinds.len(),
        model.fields.len(),
        hash
    );
    ExitCode::SUCCESS
}

struct Model {
    /// (raw type string, Rust variant ident) for every NAMED node kind, sorted by raw.
    kinds: Vec<(String, String)>,
    /// (raw field name, Rust variant ident), sorted by raw.
    fields: Vec<(String, String)>,
    hash: String,
}

impl Model {
    fn from_node_types(entries: &[serde_json::Value], hash: &str) -> Model {
        let mut kinds: Vec<(String, String)> = Vec::new();
        let mut field_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for e in entries {
            let named = e.get("named").and_then(|v| v.as_bool()).unwrap_or(false);
            let ty = e.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if named && is_ident_safe(ty) {
                kinds.push((ty.to_string(), pascal(ty)));
            }
            if let Some(fields) = e.get("fields").and_then(|v| v.as_object()) {
                for fname in fields.keys() {
                    field_set.insert(fname.clone());
                }
            }
        }
        kinds.sort();
        kinds.dedup();
        assert_no_collisions(&kinds, "RawKind");
        let fields: Vec<(String, String)> =
            field_set.into_iter().map(|f| { let p = pascal(&f); (f, p) }).collect();
        assert_no_collisions(&fields, "FieldName");
        Model { kinds, fields, hash: hash.to_string() }
    }

    fn gen_raw_kind(&self) -> String {
        let mut s = header("raw_kind.rs");
        s.push_str("/// Every NAMED node kind in the pinned tree-sitter-al grammar, plus `Error`.\n");
        s.push_str("///\n/// Exhaustive: a kind absent here cannot be produced by the pinned grammar's\n");
        s.push_str("/// named children, so [`RawKind::from_raw`] panics on an unknown string.\n");
        s.push_str("#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]\n#[allow(non_camel_case_types)]\npub enum RawKind {\n");
        for (_raw, var) in &self.kinds {
            s.push_str(&format!("    {var},\n"));
        }
        s.push_str("    /// tree-sitter error-recovery node (`kind() == \"ERROR\"`).\n    Error,\n}\n\n");

        s.push_str("impl RawKind {\n");
        s.push_str("    /// Map a tree-sitter `node.kind()` string to a `RawKind`.\n");
        s.push_str("    ///\n    /// LOUD on unknown: only NAMED kinds are passed here (the lowerer classifies\n");
        s.push_str("    /// named children); an unknown string means the binary was built against a\n");
        s.push_str("    /// different grammar than it is parsing — a bug, not data.\n");
        s.push_str("    pub fn from_raw(s: &str) -> RawKind {\n        match s {\n");
        for (raw, var) in &self.kinds {
            s.push_str(&format!("            {raw:?} => RawKind::{var},\n"));
        }
        s.push_str("            \"ERROR\" => RawKind::Error,\n");
        s.push_str("            other => panic!(\n                \"al-syntax: unknown node kind {other:?} — grammar/binary mismatch \\\n                 (regenerate with `cargo run -p xtask -- gen-syntax`)\"\n            ),\n        }\n    }\n\n");

        s.push_str("    /// The grammar kind string (round-trips `from_raw`).\n");
        s.push_str("    pub fn as_str(self) -> &'static str {\n        match self {\n");
        for (raw, var) in &self.kinds {
            s.push_str(&format!("            RawKind::{var} => {raw:?},\n"));
        }
        s.push_str("            RawKind::Error => \"ERROR\",\n        }\n    }\n}\n\n");

        s.push_str(&format!(
            "/// sha256 of the `node-types.json` this file was generated from. al-syntax\n/// `build.rs` asserts the checked-in grammar matches, so a silent grammar swap\n/// fails the build.\npub const GRAMMAR_NODE_TYPES_HASH: &str = {:?};\n\n",
            self.hash
        ));
        s.push_str(&format!(
            "/// Count of NAMED kinds (excludes `Error`). Sanity anchor for the coverage test.\npub const NAMED_KIND_COUNT: usize = {};\n",
            self.kinds.len()
        ));
        s
    }

    fn gen_field(&self) -> String {
        let mut s = header("field.rs");
        s.push_str("/// Every field name in the pinned tree-sitter-al grammar.\n");
        s.push_str("///\n/// A renamed/removed field drops its variant here, so every `FieldName::X`\n/// call site fails to compile (the loud path for field drift).\n");
        s.push_str("#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]\n#[allow(non_camel_case_types)]\npub enum FieldName {\n");
        for (_raw, var) in &self.fields {
            s.push_str(&format!("    {var},\n"));
        }
        s.push_str("}\n\nimpl FieldName {\n    /// The grammar field-name string (for `child_by_field_name`).\n");
        s.push_str("    pub fn as_raw(self) -> &'static str {\n        match self {\n");
        for (raw, var) in &self.fields {
            s.push_str(&format!("            FieldName::{var} => {raw:?},\n"));
        }
        s.push_str("        }\n    }\n}\n");
        s
    }

    fn gen_mod(&self) -> String {
        let mut s = header("mod.rs");
        s.push_str("//! Generated raw grammar vocabulary. Regenerate with\n//! `cargo run -p xtask -- gen-syntax`; CI runs `--check` to catch drift.\n\n");
        s.push_str("mod field;\nmod raw_kind;\n\npub use field::FieldName;\npub use raw_kind::{RawKind, GRAMMAR_NODE_TYPES_HASH, NAMED_KIND_COUNT};\n");
        s
    }
}

fn header(file: &str) -> String {
    format!(
        "// @generated by `cargo run -p xtask -- gen-syntax` from tree-sitter-al\n// node-types.json. DO NOT EDIT — edit the generator or bump the grammar instead.\n// Source file: crates/al-syntax/src/raw/generated/{file}\n\n"
    )
}

/// snake_case / lowercase grammar string → PascalCase Rust ident.
fn pascal(s: &str) -> String {
    let mut out = String::new();
    for part in s.split('_') {
        let mut c = part.chars();
        if let Some(first) = c.next() {
            out.extend(first.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    out
}

fn is_ident_safe(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn assert_no_collisions(pairs: &[(String, String)], what: &str) {
    let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (raw, var) in pairs {
        if let Some(prev) = seen.insert(var, raw) {
            panic!("{what} ident collision: {prev:?} and {raw:?} both → {var}");
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Strip CR for a line-ending-agnostic comparison (Windows checkouts may be CRLF).
fn normalize(s: &str) -> String {
    s.replace('\r', "")
}

fn workspace_root() -> PathBuf {
    // xtask manifest dir = <ws>/crates/xtask
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|p| p.join("Cargo.toml").exists() && p.join("crates").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest.join("../.."))
}
