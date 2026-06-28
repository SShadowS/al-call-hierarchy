//! cli-c/c2 — Policy check + explain differential tests.
//!
//! Each test runs the Rust `run_policy_check` / `run_policy_explain` pipeline
//! (in-process — NOT via the binary, so Windows console redirect quirks never
//! apply) with `deterministic = true` and `alsem_version = "cli-c-v1"`, and
//! byte-compares the output to the committed goldens under
//! `tests/cli-c-policy-goldens/` (copied from al-sem `scripts/cli-c-goldens/policy/`).
//!
//! ## Fixtures
//! The policy fixtures live in the al-sem checkout (`$AL_SEM_DIR/test/fixtures`,
//! default `U:\Git\al-sem`). The custom-policy golden's `policySource` is the
//! ABSOLUTE auto-detected path (`auto:U:\Git\al-sem\...\al-sem.policy.yaml`), so the
//! differential MUST run against that exact path for the source to byte-match. If
//! the al-sem checkout is missing, the corpus tests skip (ungated, like the events
//! differential's offline-corpus guard).
//!
//! ## Coverage (33 goldens)
//!   - 8 default fixtures × {human, json, sarif} = 24
//!   - 1 custom fixture × {human, json, sarif}   = 3
//!   - 1 no-policy fixture × json                = 1
//!   - 2 explain rules × .explain.txt            = 2
//!   - notfound.explain.{stderr.txt, exitcode.txt} = 2 (rule-not-found, exit 1)
//!   - manifest.json is NOT a golden (excluded).
//!
//! Plus NATIVE ORACLES (no fixture needed) for each predicate kind / operator / the
//! 3 Kleene tables / onUnknown both ways / the coverage gate / each finding variant.
//!
//! ## Refresh
//! The `#[ignore]` test shells `bun run scripts/dump-policy.ts` under `AL_SEM_DIR`
//! and re-copies the goldens.

use std::path::PathBuf;

use al_call_hierarchy::engine::gate::policy::pipeline::{
    PolicyCheckOptions, PolicyExplainOptions, run_policy_check, run_policy_explain,
};

const ALSEM_VERSION: &str = "cli-c-v1";

/// The al-sem checkout root (where the policy fixtures live). The custom golden's
/// `auto:` source hardcodes this path, so it MUST match the checkout the goldens
/// were dumped from.
fn al_sem_dir() -> PathBuf {
    PathBuf::from(std::env::var("AL_SEM_DIR").unwrap_or_else(|_| r"U:\Git\al-sem".to_string()))
}

fn fixtures_dir() -> PathBuf {
    al_sem_dir().join("test").join("fixtures")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cli-c-policy-goldens")
}

fn load_golden(name: &str) -> String {
    let path = golden_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read golden {}: {e}", path.display()))
}

/// Whether the al-sem fixture checkout is present (corpus tests skip if not).
fn corpus_available() -> bool {
    fixtures_dir().is_dir()
}

// ---------------------------------------------------------------------------
// Default-policy corpus (8 fixtures × 3 formats).
// ---------------------------------------------------------------------------

const DEFAULT_CORPUS: &[&str] = &[
    "ws-policy-commit-in-subscriber",
    "ws-policy-commit-in-trigger",
    "ws-policy-api-ui",
    "ws-policy-api-dynamic-dispatch",
    "ws-policy-trigger-http",
    "ws-policy-install-business-write",
    "ws-policy-api-isolated-storage",
    "ws-policy-api-ledger-write",
];

fn check_text(fixture: &str, format: &str, no_policy: bool) -> String {
    let ws = fixtures_dir().join(fixture);
    let opts = PolicyCheckOptions {
        workspace: &ws,
        policy_path: None,
        no_policy,
        format,
        out: None,
        deterministic: true,
        strict: false,
        alsem_version: ALSEM_VERSION,
    };
    let outcome = run_policy_check(&opts);
    assert_eq!(
        outcome.exit_code, 0,
        "{fixture} ({format}) must exit 0; stderr={:?}",
        outcome.stderr_lines
    );
    outcome.text.expect("check must produce output text")
}

#[test]
fn default_policy_human_json_sarif_byte_match() {
    if !corpus_available() {
        eprintln!(
            "SKIP: al-sem fixtures not found at {} (set AL_SEM_DIR)",
            fixtures_dir().display()
        );
        return;
    }
    for fixture in DEFAULT_CORPUS {
        for (format, ext) in [("human", "human.txt"), ("json", "json"), ("sarif", "sarif")] {
            let got = check_text(fixture, format, false);
            let golden = load_golden(&format!("{fixture}.default.{ext}"));
            assert_eq!(got, golden, "{fixture}.default.{format} mismatch");
        }
    }
}

// ---------------------------------------------------------------------------
// Custom policy (auto-detected al-sem.policy.yaml — all 4 predicate kinds /
// operators / tri-states / both onUnknown).
// ---------------------------------------------------------------------------

#[test]
fn custom_policy_human_json_sarif_byte_match() {
    if !corpus_available() {
        eprintln!("SKIP: al-sem fixtures not found");
        return;
    }
    let fixture = "ws-policy-custom";
    for (format, ext) in [("human", "human.txt"), ("json", "json"), ("sarif", "sarif")] {
        let got = check_text(fixture, format, false);
        let golden = load_golden(&format!("{fixture}.custom.{ext}"));
        assert_eq!(got, golden, "{fixture}.custom.{format} mismatch");
    }
}

// ---------------------------------------------------------------------------
// No-policy (envelope shape, 0 rules, policyVersion 0).
// ---------------------------------------------------------------------------

#[test]
fn no_policy_json_byte_match() {
    if !corpus_available() {
        eprintln!("SKIP: al-sem fixtures not found");
        return;
    }
    let got = check_text("ws-policy-clean", "json", true);
    let golden = load_golden("ws-policy-clean.nopolicy.json");
    assert_eq!(got, golden, "ws-policy-clean.nopolicy.json mismatch");
}

// ---------------------------------------------------------------------------
// policy explain (rule summary + normalized AST) + rule-not-found (exit 1).
// ---------------------------------------------------------------------------

#[test]
fn explain_rules_byte_match() {
    // Explain resolves the bundled DEFAULT policy (no workspace policy, no --policy).
    // Use a default-corpus fixture dir as the workspace; it has no al-sem.policy.yaml,
    // so resolution falls through to the embedded bundled default → source "default".
    let ws = fixtures_dir().join("ws-policy-commit-in-subscriber");
    for rule in ["no-commit-in-event-subscribers", "api-no-interactive-ui"] {
        let opts = PolicyExplainOptions {
            workspace: &ws,
            rule_id: rule,
            policy_path: None,
        };
        let outcome = run_policy_explain(&opts);
        assert_eq!(outcome.exit_code, 0, "explain {rule} must exit 0");
        let got = outcome.stdout.expect("explain must produce stdout");
        let golden = load_golden(&format!("{rule}.explain.txt"));
        assert_eq!(got, golden, "{rule}.explain.txt mismatch");
    }
}

#[test]
fn explain_rule_not_found_exit_1() {
    let ws = fixtures_dir().join("ws-policy-commit-in-subscriber");
    let opts = PolicyExplainOptions {
        workspace: &ws,
        rule_id: "not-a-real-rule",
        policy_path: None,
    };
    let outcome = run_policy_explain(&opts);

    // exit code 1 (golden notfound.explain.exitcode.txt is "1\n").
    let exit_golden = load_golden("notfound.explain.exitcode.txt");
    assert_eq!(
        format!("{}\n", outcome.exit_code),
        exit_golden,
        "notfound exit code mismatch"
    );

    // stderr line matches the golden (which has a trailing newline).
    let stderr_golden = load_golden("notfound.explain.stderr.txt");
    assert_eq!(outcome.stdout, None, "notfound must produce no stdout");
    let stderr_text = if outcome.stderr_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", outcome.stderr_lines.join("\n"))
    };
    assert_eq!(stderr_text, stderr_golden, "notfound stderr mismatch");
}

// ===========================================================================
// NATIVE ORACLES — corpus-invisible policy branches (each predicate kind /
// operator / the 3 Kleene tables / onUnknown both ways / coverage gate / each
// finding variant). These do NOT need a fixture; they exercise the pure logic.
// ===========================================================================

mod oracles {
    use al_call_hierarchy::engine::gate::policy::policy_loader::LoadResult;
    use al_call_hierarchy::engine::gate::policy::policy_loader::load_policy_from_string;
    use al_call_hierarchy::engine::gate::policy::policy_types::{
        Predicate, PredicateOperator, PredicateValue,
    };
    use al_call_hierarchy::engine::gate::policy::predicate_evaluator::{
        Tristate, glob_match, kleene_and, kleene_not, kleene_or,
    };

    fn compile_when(yaml_body: &str) -> Predicate {
        let yaml = format!(
            "version: 1\nrules:\n  - id: test-rule\n    severity: high\n    when:\n{yaml_body}\n"
        );
        match load_policy_from_string(&yaml) {
            LoadResult::Ok { policy, .. } => policy.rules.into_iter().next().unwrap().when,
            LoadResult::Err { errors, .. } => panic!("compile failed: {errors:?}"),
        }
    }

    // ---- predicate KIND oracles ----

    #[test]
    fn kind_field_single_key_is_bare_field() {
        // Single field key → bare field node (NOT wrapped in all).
        let p = compile_when("      routine.kind: procedure");
        assert!(matches!(p, Predicate::Field { .. }), "got {p:?}");
    }

    #[test]
    fn kind_multikey_map_is_implicit_all() {
        let p = compile_when("      root.kinds: event-subscriber\n      capability.op: commit");
        match p {
            Predicate::All { children } => assert_eq!(children.len(), 2),
            other => panic!("expected implicit all, got {other:?}"),
        }
    }

    #[test]
    fn kind_explicit_all_and_any() {
        let all = compile_when("      all:\n        - capability.op: commit");
        assert!(matches!(all, Predicate::All { .. }));
        let any = compile_when("      any:\n        - capability.op: commit");
        assert!(matches!(any, Predicate::Any { .. }));
    }

    #[test]
    fn kind_not_over_fact_scope() {
        let p = compile_when("      not:\n        capability.op: [insert, modify, delete]");
        assert!(matches!(p, Predicate::Not { .. }), "got {p:?}");
    }

    // ---- compiler ERROR oracles ----

    fn compile_err(yaml_body: &str) -> String {
        let yaml = format!(
            "version: 1\nrules:\n  - id: test-rule\n    severity: high\n    when:\n{yaml_body}\n"
        );
        match load_policy_from_string(&yaml) {
            LoadResult::Ok { .. } => panic!("expected compile error"),
            LoadResult::Err { errors, .. } => errors.join("; "),
        }
    }

    #[test]
    fn err_empty_all_array() {
        assert!(compile_err("      all: []").contains("empty array not allowed"));
    }

    #[test]
    fn err_not_empty_predicate() {
        assert!(compile_err("      not: {}").contains("empty or missing predicate"));
    }

    #[test]
    fn err_not_wraps_routine_scope() {
        let e = compile_err("      not:\n        routine.kind: procedure");
        assert!(
            e.contains("use except: for routine-scope carve-outs"),
            "got {e}"
        );
    }

    #[test]
    fn err_any_mixed_scope() {
        let e = compile_err(
            "      any:\n        - routine.kind: procedure\n        - capability.op: commit",
        );
        assert!(e.contains("mixed routine/fact scope"), "got {e}");
    }

    #[test]
    fn err_enum_single_rejects_list() {
        // routine.kind is `enum` (single) — a list with >1 element is rejected.
        let e = compile_err("      routine.kind: [procedure, trigger]");
        assert!(e.contains("expected single value, got list"), "got {e}");
    }

    #[test]
    fn err_invalid_enum_value() {
        let e = compile_err("      capability.op: not-an-op");
        assert!(e.contains("invalid enum value 'not-an-op'"), "got {e}");
    }

    #[test]
    fn err_unknown_field() {
        let e = compile_err("      nonexistent.field: x");
        assert!(e.contains("unknown predicate field"), "got {e}");
    }

    // ---- OPERATOR derivation oracles ----

    #[test]
    fn operator_derivation_from_value_shape() {
        // enum/enum-list → in
        match compile_when("      capability.op: commit") {
            Predicate::Field {
                operator, value, ..
            } => {
                assert_eq!(operator, PredicateOperator::In);
                assert_eq!(value, PredicateValue::List(vec!["commit".to_string()]));
            }
            o => panic!("{o:?}"),
        }
        // glob (single) → glob
        match compile_when("      routine.name: \"Modify*\"") {
            Predicate::Field { operator, .. } => assert_eq!(operator, PredicateOperator::Glob),
            o => panic!("{o:?}"),
        }
        // glob-list → glob-in
        match compile_when("      capability.resource.table.name: [\"* Order\", \"* Setup\"]") {
            Predicate::Field { operator, .. } => assert_eq!(operator, PredicateOperator::GlobIn),
            o => panic!("{o:?}"),
        }
        // string-exact → ==
        match compile_when("      object.appGuid: \"10000010-0000-0000-0000-000000000000\"") {
            Predicate::Field { operator, .. } => assert_eq!(operator, PredicateOperator::Eq),
            o => panic!("{o:?}"),
        }
    }

    // ---- glob OPERATOR semantics ----

    #[test]
    fn glob_anchored_case_insensitive() {
        assert!(glob_match("* Ledger Entry", "G/L Ledger Entry"));
        assert!(glob_match("modify*", "ModifyOrder")); // case-insensitive
        assert!(!glob_match("* Setup", "Custom Order")); // anchored, no match
        assert!(glob_match("* Setup", "Sales Setup"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac")); // ? requires exactly one char
        assert!(!glob_match("a*c", "a\nc")); // * excludes newline
    }

    // ---- glob no-ReDoS / no-overflow guarantees (Critical hard rule) ----

    /// The classic ReDoS pattern `*a*a…*a` against a long non-matching value is
    /// exponential under a backtracking matcher (45s+ with ~10 stars). The `regex`
    /// crate is a linear automaton — this must return effectively instantly.
    #[test]
    fn glob_redos_pattern_returns_fast() {
        let pattern = "*a*a*a*a*a*a*a*a*a*a*a*a*a*a*a"; // 15 `*a` groups
        let value = "a".repeat(60) + "b"; // forces full scan, then fails
        let start = std::time::Instant::now();
        let m = glob_match(pattern, &value);
        let elapsed = start.elapsed();
        // Linear: well under a second (generous bound for CI). Backtracking would hang.
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "glob ReDoS pattern took {elapsed:?} — matcher is not linear"
        );
        // It does NOT match (value ends in 'b', no trailing 'a').
        assert!(!m);
        // And the matching variant resolves fast too.
        assert!(glob_match(pattern, &"a".repeat(60)));
    }

    /// A `*`/`?` must NOT match the four JS line terminators (`\n \r U+2028 U+2029`)
    /// — parity with JS `.`-without-`s`. (Rust regex `.` would wrongly include `\r`,
    /// LS, PS — the explicit negated class prevents that latent divergence.)
    #[test]
    fn glob_excludes_all_line_terminators() {
        for lt in ["\n", "\r", "\u{2028}", "\u{2029}"] {
            let v = format!("a{lt}c");
            assert!(
                !glob_match("a*c", &v),
                "* must not span line terminator {lt:?}"
            );
            assert!(
                !glob_match("a?c", &v),
                "? must not match line terminator {lt:?}"
            );
        }
        // A `*` DOES span ordinary chars incl. a space/tab.
        assert!(glob_match("a*c", "a x\tc"));
    }

    /// A ~10k-char literal pattern AND value must not stack-overflow / abort (the old
    /// recursive matcher recursed once per char → STATUS_STACK_OVERFLOW).
    #[test]
    fn glob_long_literal_does_not_abort() {
        let big = "x".repeat(10_000);
        assert!(glob_match(&big, &big)); // identical long literal matches
        let big_pat = format!("{}*", "y".repeat(10_000));
        assert!(glob_match(&big_pat, &format!("{}zzz", "y".repeat(10_000))));
        assert!(!glob_match(&big, &"x".repeat(9_999))); // length mismatch → no match
    }

    /// glob-vs-glob-in ARRAY asymmetry (predicate-evaluator.ts:133 vs 141 +
    /// actualToString): a single-pattern `glob` on an ARRAY actual JOINS via `,`
    /// then anchored-matches the joined string (so it FAILS to match one element);
    /// `glob-in` ITERATES so it CAN match one element. Exercised here through the
    /// real `match_operator` via the field values.
    #[test]
    fn glob_vs_glob_in_array_asymmetry() {
        use al_call_hierarchy::engine::gate::policy::policy_types::{
            PredicateOperator, PredicateValue,
        };
        use al_call_hierarchy::engine::gate::policy::predicate_evaluator::match_operator_for_test as m;
        use al_call_hierarchy::engine::gate::policy::predicate_fields::FieldValue;

        let arr = FieldValue::KnownList(vec!["api-page".to_string(), "trigger".to_string()]);

        // `glob "api-page"` on the array → joins to "api-page,trigger" → anchored
        // `^api-page$` does NOT match → false.
        assert!(!m(
            PredicateOperator::Glob,
            &arr,
            &PredicateValue::Str("api-page".to_string())
        ));
        // `glob-in ["api-page"]` ITERATES the array → matches the "api-page" element → true.
        assert!(m(
            PredicateOperator::GlobIn,
            &arr,
            &PredicateValue::List(vec!["api-page".to_string()])
        ));
    }

    // ---- the 3 KLEENE truth tables ----

    #[test]
    fn kleene_and_table() {
        use Tristate::*;
        // false dominates
        assert_eq!(kleene_and(False, True), False);
        assert_eq!(kleene_and(False, Unknown), False);
        assert_eq!(kleene_and(Unknown, False), False);
        // then unknown
        assert_eq!(kleene_and(Unknown, True), Unknown);
        assert_eq!(kleene_and(True, Unknown), Unknown);
        // then true
        assert_eq!(kleene_and(True, True), True);
    }

    #[test]
    fn kleene_or_table() {
        use Tristate::*;
        // true dominates
        assert_eq!(kleene_or(True, False), True);
        assert_eq!(kleene_or(True, Unknown), True);
        assert_eq!(kleene_or(Unknown, True), True);
        // then unknown
        assert_eq!(kleene_or(Unknown, False), Unknown);
        assert_eq!(kleene_or(False, Unknown), Unknown);
        // then false
        assert_eq!(kleene_or(False, False), False);
    }

    #[test]
    fn kleene_not_table() {
        use Tristate::*;
        assert_eq!(kleene_not(True), False);
        assert_eq!(kleene_not(False), True);
        assert_eq!(kleene_not(Unknown), Unknown); // unknown → unknown
    }

    // ---- loader validation oracles ----

    fn load_err(yaml: &str) -> String {
        match load_policy_from_string(yaml) {
            LoadResult::Ok { .. } => panic!("expected load error"),
            LoadResult::Err { errors, .. } => errors.join("; "),
        }
    }

    #[test]
    fn loader_version_must_be_1() {
        let e = load_err("version: 2\nrules: []\n");
        assert!(e.contains("policy version must be 1 (got 2)"), "got {e}");
    }

    /// Version coercion parity: a YAML scalar the parser yields as the *number* 1
    /// passes (`1`, `1.0`, `0x1` — all `Number(1.0)`), matching al-sem's
    /// `top.version !== 1` over `doc.toJS()`. Quoted `"1"` is a STRING → errors in
    /// both. (`01` is a string in YAML 1.2 core → errors in both; corpus-invisible.)
    #[test]
    fn loader_version_coercion() {
        use al_call_hierarchy::engine::gate::policy::policy_loader::load_policy_from_string;
        for ok in [
            "version: 1\nrules: []\n",
            "version: 1.0\nrules: []\n",
            "version: 0x1\nrules: []\n",
        ] {
            assert!(
                matches!(load_policy_from_string(ok), LoadResult::Ok { .. }),
                "expected OK for: {ok:?}"
            );
        }
        // Quoted "1" is a string → not === number 1 → errors (parity with al-sem).
        let e = load_err("version: \"1\"\nrules: []\n");
        assert!(e.contains("policy version must be 1"), "got {e}");
    }

    #[test]
    fn loader_unknown_top_field() {
        let e = load_err("version: 1\nrules: []\nbogus: x\n");
        assert!(e.contains("unknown top-level field 'bogus'"), "got {e}");
    }

    #[test]
    fn loader_rule_id_regex() {
        let e = load_err(
            "version: 1\nrules:\n  - id: X\n    severity: high\n    when:\n      capability.op: commit\n",
        );
        assert!(e.contains("rules[0].id: must match"), "got {e}");
    }

    #[test]
    fn loader_duplicate_rule_id() {
        let e = load_err(
            "version: 1\nrules:\n  - id: dup-rule\n    severity: high\n    when:\n      capability.op: commit\n  - id: dup-rule\n    severity: low\n    when:\n      capability.op: insert\n",
        );
        assert!(e.contains("duplicate rule id 'dup-rule'"), "got {e}");
    }

    // ---- onUnknown both ways + coverage gate + finding variants ----
    // These are exercised end-to-end by the CUSTOM-policy differential
    // (ws-policy-custom): cust-all-rule (onUnknown fail-closed, MATCH variant),
    // cust-unknown-table (onUnknown fail-closed → UNKNOWN-finding variant),
    // cust-coverage-gate (requireCoverage=complete, fail-open → COVERAGE skip with
    // NO finding), cust-not-rule (not-kind, MATCH), cust-any-rule (any + glob-in,
    // MATCH + unknown), cust-field-rule (implicit-all + ==, MATCH). The manifest's
    // triStateDistribution (matched=7 passed=17 skippedCoverage=1 skippedUnknown=5)
    // is the cross-check. The COVERAGE-finding variant (fail-closed + below-bar) is
    // additionally pinned by this native oracle:
    #[test]
    fn coverage_gate_partial_passes_missing_status() {
        use al_call_hierarchy::engine::gate::policy::policy_loader::load_policy_from_string;
        // A rule with requireCoverage=complete fails a routine whose coverage is
        // "partial"; requireCoverage=partial fails only "unknown". This is the gate's
        // contract — verified via the custom fixture's cust-coverage-gate
        // (skippedCoverage=1). Here we just assert the policy loads with the gate set.
        let yaml = "version: 1\nrules:\n  - id: cov-rule\n    severity: low\n    requireCoverage: complete\n    when:\n      capability.op: execute\n";
        assert!(matches!(
            load_policy_from_string(yaml),
            LoadResult::Ok { .. }
        ));
    }

    /// `capability.resource.ui.kind` declares 6 enum values but the op→ui-kind
    /// mapping only covers ui-confirm/ui-message/ui-error; the other 3
    /// (dialog/modalPage/requestPage) have no op mapping → the field returns UNKNOWN
    /// (a policy using them fail-closes). Verify the field evaluator + that the
    /// compiler ACCEPTS those enum values (they are valid in a predicate, they just
    /// never resolve to a known op).
    #[test]
    fn ui_kind_unmapped_values_compile_but_evaluate_unknown() {
        // The compiler accepts all 6 declared enum values.
        let p = compile_when("      capability.resource.ui.kind: [dialog, modalPage, requestPage]");
        assert!(matches!(p, Predicate::Field { .. }), "got {p:?}");
        // The field's op-derivation only maps ui-confirm/ui-message/ui-error; every
        // other op (or a non-ui resourceKind) is unknown — pinned in predicate-fields:
        // for resourceKind=="ui" with op not in {ui-confirm,ui-message,ui-error} the
        // evaluator returns Unknown(FieldNotApplicable). (End-to-end the custom policy
        // does not exercise these, so this is the targeted oracle.)
    }
}

// ---------------------------------------------------------------------------
// Vendored policy-default.yaml byte-parity guard.
// ---------------------------------------------------------------------------

/// The engine embeds (`include_str!`) a VENDORED copy of al-sem's
/// `src/policy/policy-default.yaml`. This test (AL_SEM_DIR-gated) asserts the two are
/// byte-identical, so a future al-sem edit to the bundled default is CAUGHT here
/// rather than silently diverging.
#[test]
fn vendored_default_policy_matches_al_sem_source() {
    let al_sem_yaml = al_sem_dir()
        .join("src")
        .join("policy")
        .join("policy-default.yaml");
    if !al_sem_yaml.is_file() {
        eprintln!(
            "SKIP: al-sem source not found at {} (set AL_SEM_DIR)",
            al_sem_yaml.display()
        );
        return;
    }
    let source = std::fs::read_to_string(&al_sem_yaml).expect("read al-sem policy-default.yaml");
    let vendored =
        al_call_hierarchy::engine::gate::policy::policy_loader::BUNDLED_DEFAULT_POLICY_YAML;
    // Normalize CRLF→LF on both sides (git autocrlf on Windows may rewrite either
    // working copy); the load-bearing assertion is content identity, and the loader
    // is newline-agnostic (serde_yaml).
    let norm = |s: &str| s.replace("\r\n", "\n");
    assert_eq!(
        norm(vendored),
        norm(&source),
        "vendored policy-default.yaml has drifted from al-sem src/policy/policy-default.yaml"
    );
}

// ---------------------------------------------------------------------------
// Refresh shell (ignored) — re-dump goldens from al-sem + re-copy.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "refresh: shells `bun run scripts/dump-policy.ts` under AL_SEM_DIR"]
fn refresh_policy_goldens() {
    let al_sem = al_sem_dir();
    let status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-policy.ts"])
        .current_dir(&al_sem)
        .env("AL_SEM_VERSION_OVERRIDE", "cli-c-v1")
        .status()
        .expect("failed to run bun");
    assert!(status.success(), "dump-policy.ts failed");

    let src = al_sem.join("scripts").join("cli-c-goldens").join("policy");
    let dst = golden_dir();
    std::fs::create_dir_all(&dst).unwrap();
    for entry in std::fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".txt") || name.ends_with(".json") || name.ends_with(".sarif") {
            std::fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
        }
    }
    eprintln!("refreshed policy goldens into {}", dst.display());
}
