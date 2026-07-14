//! Privacy lint: `src/telemetry/events.rs` must not introduce raw-string fields
//! that could carry unhashed AL identifiers. Allowed `String` fields are those
//! whose names end in `_hash`, plus a small allowlist for non-identifier strings
//! (file_extension, ts_node_path, method, install_id, workspace_id).
//!
//! This is a hard CI gate. If you legitimately need a new `String` field, add
//! it to ALLOWED_FIELDS below with a justification.

use std::fs;

const ALLOWED_FIELDS: &[&str] = &[
    "file_extension",  // "al" / "dal"; closed enum encoded as String
    "ts_node_path",    // tree-sitter grammar shape; public
    "method",          // &'static str, but match is permissive
    "install_id",      // hashed in production; allowed
    "workspace_id",    // hashed; allowed
    "al_version",      // &'static str
    "grammar_version", // &'static str
    "os",              // &'static str
];

#[test]
fn no_unhashed_string_fields_in_events_module() {
    let src = fs::read_to_string("src/telemetry/events.rs")
        .expect("events.rs must exist for privacy lint");

    // Collect every line that declares `pub xxx: String` or `pub xxx: Option<String>`.
    let mut violations = Vec::new();
    for (lineno, line) in src.lines().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("pub ") else {
            continue;
        };
        let Some(colon_pos) = rest.find(':') else {
            continue;
        };
        let field_name = rest[..colon_pos].trim();
        let type_part = rest[colon_pos + 1..].trim();

        let is_string = type_part.starts_with("String")
            || type_part.starts_with("Option<String>")
            || type_part.starts_with("Option<&");

        if !is_string {
            continue;
        }
        // Allowed if the name ends with `_hash` or is in the allowlist.
        if field_name.ends_with("_hash") {
            continue;
        }
        if ALLOWED_FIELDS.contains(&field_name) {
            continue;
        }
        violations.push(format!(
            "L{}: field `{}` of type {}",
            lineno + 1,
            field_name,
            type_part
        ));
    }

    assert!(
        violations.is_empty(),
        "Privacy lint violations in src/telemetry/events.rs (raw String fields that may leak identifiers):\n{}\n\n\
        If a new field is intentionally a non-identifier String, add it to ALLOWED_FIELDS in this test with a justification.",
        violations.join("\n")
    );
}
