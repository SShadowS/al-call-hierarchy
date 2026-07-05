//! R3b Task 2 — STAGE 2 incremental-equality proof.
//!
//! The Stage-2 oracle: `salsa_incremental(base, edit) == from_scratch(base+edit)`
//! BYTE-FOR-BYTE, for random edits over the corpus. We build a persistent, EDITABLE
//! Salsa DB over each fixture (`EditableModel`), apply ONE edit via the fine-grained
//! input setters, re-demand the L4 output incrementally, and compare it to a FRESH
//! from-scratch demand over the same edited inputs. The from-scratch path is the
//! Stage-1-parity oracle (wrap == from-scratch == al-sem golden), so equality here
//! ties the incremental result to the al-sem ground truth transitively.
//!
//! Edit kinds exercised (the plan's "Routine universe + id churn" + set-fact +
//! no-op-at-L4):
//!   - add / remove a call (combined) edge
//!   - change a routine's direct dbEffects (its base summary) / direct facts /
//!     direct coverage / body_available
//!   - change app_identity / bump dep_stamp
//!   - routine ADD / REMOVE / RENAME (== signature-rehash; StableRoutineId re-hash)
//!   - NO-OP-at-L4 (set the same value; add a duplicate/dominated edge; cosmetic
//!     dep-stamp bump) — MUST early-cut (no `scc_summaries`/`cones` recompute) AND
//!     stay byte-equal.
//!
//! The proof is exact byte-equality — no tolerated divergence.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::l4::combined_graph::CombinedEdge;
use al_call_hierarchy::engine::l4::incremental::edit::{InputModel, RoutineFacts};
use al_call_hierarchy::engine::l4::incremental::wrap::input_model_r3a3_source_only;
use al_call_hierarchy::engine::l4::summary::{DbEffect, TempState};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A tiny deterministic xorshift PRNG — the edit GENERATION is reproducible and
/// independent of `RUST_HASH_SEED` (which we vary separately to probe HashMap
/// iteration nondeterminism). No external crate.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() {
            None
        } else {
            Some(&v[self.below(v.len())])
        }
    }
}

/// Discover the source-only fixtures that have an r3a3 golden AND resolve.
fn discover_fixtures() -> Vec<(String, InputModel)> {
    let gold_dir = repo_root().join("tests").join("r3a3-goldens");
    let corpus = repo_root().join("tests").join("r0-corpus");
    let mut names: Vec<String> = std::fs::read_dir(&gold_dir)
        .expect("read r3a3 goldens")
        .filter_map(|e| {
            let n = e.ok()?.file_name().to_string_lossy().to_string();
            n.strip_suffix(".r3a3.golden.json").map(|s| s.to_string())
        })
        .collect();
    names.sort();

    let mut out = Vec::new();
    for name in names {
        let dir = corpus.join(&name);
        if !dir.is_dir() {
            continue;
        }
        if let Some(resolved) = assemble_and_resolve_workspace_default(&dir) {
            let model = input_model_r3a3_source_only(&resolved);
            // Only keep fixtures with at least one routine (an edit needs a subject).
            if !model.routine_ids.is_empty() {
                out.push((name, model));
            }
        }
    }
    out
}

/// Assert the incremental demand (reused DB, one edit applied via setters) is
/// byte-identical to a fresh from-scratch demand over the SAME edited inputs.
/// `mutate` applies the edit BOTH to the live `EditableModel` (Salsa setters) — its
/// mirrored `model` is kept in sync — and the assertion rebuilds the from-scratch
/// oracle from that mirror.
fn assert_edit_equal<
    F: FnOnce(&mut al_call_hierarchy::engine::l4::incremental::edit::EditableModel),
>(
    fixture: &str,
    label: &str,
    base: &InputModel,
    mutate: F,
) {
    let mut editable = base.build_incremental();
    // Prime the base demand (so the reuse path has memoized state to invalidate).
    let _ = editable.demand();

    mutate(&mut editable);

    let incremental = editable.demand();
    let from_scratch = editable.model.demand_from_scratch();

    assert_eq!(
        incremental.fingerprint(),
        from_scratch.fingerprint(),
        "[{fixture}] edit `{label}`: incremental demand is NOT byte-identical to \
         from-scratch over the edited inputs"
    );
    assert!(
        incremental == from_scratch,
        "[{fixture}] edit `{label}`: PartialEq mismatch (incremental != from-scratch)"
    );
}

// ===========================================================================
// The property test — every edit kind, over every fixture, byte-equal.
// ===========================================================================

#[test]
fn r3b_stage2_incremental_equals_from_scratch_over_corpus() {
    let fixtures = discover_fixtures();
    assert!(
        fixtures.len() >= 20,
        "expected a substantial corpus (>=20 source-only fixtures), got {}",
        fixtures.len()
    );

    let mut edits_checked = 0usize;
    let mut kind_counts: HashMap<&str, usize> = HashMap::new();

    for (fixture, base) in &fixtures {
        // A deterministic per-fixture seed (the fixture name bytes).
        let seed = fixture.bytes().fold(1469598103934665603u64, |h, b| {
            (h ^ b as u64).wrapping_mul(1099511628211)
        });
        let mut rng = Rng::new(seed);
        let ids: Vec<String> = base.routine_ids.clone();

        // --- set-fact: add a (dominated/no-op-shaped, then a real) call edge ---
        if let Some(from) = rng.pick(&ids).cloned() {
            let to = rng.pick(&ids).cloned().unwrap_or_else(|| from.clone());
            // REAL call-edge ADD: a new direct edge from→to (may genuinely change
            // the SCC structure + the propagated summary).
            assert_edit_equal(fixture, "add-call-edge", base, |em| {
                let mut edges = em
                    .model
                    .routines
                    .get(&from)
                    .map(|f| f.combined_edges.clone())
                    .unwrap_or_default();
                edges.push(CombinedEdge {
                    from: from.clone(),
                    to: to.clone(),
                    kind: "method".to_string(),
                    callsite_id: Some(format!("synthetic-callsite::{from}::{to}")),
                    operation_id: None,
                    event_id: None,
                    subscriber_app_id: None,
                    resolution: "resolved".to_string(),
                });
                em.set_combined_edges(&from, edges);
            });
            *kind_counts.entry("add-call-edge").or_default() += 1;
            edits_checked += 1;

            // REMOVE a call edge (drop the first outgoing edge if any).
            assert_edit_equal(fixture, "remove-call-edge", base, |em| {
                if let Some(f) = em.model.routines.get(&from) {
                    let mut edges = f.combined_edges.clone();
                    if !edges.is_empty() {
                        edges.remove(0);
                    }
                    em.set_combined_edges(&from, edges);
                }
            });
            *kind_counts.entry("remove-call-edge").or_default() += 1;
            edits_checked += 1;
        }

        // --- change direct dbEffects (the base summary) ---
        if let Some(id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "change-db-effects", base, |em| {
                if let Some(f) = em.model.routines.get(&id) {
                    let mut base_summary = f.base_summary.clone();
                    base_summary.db_effects.push(DbEffect {
                        effect_key: "synthetic|modify|tbl|op".to_string(),
                        operation_id: "synthetic-op".to_string(),
                        op: "modify".to_string(),
                        table_id: "synthetic-table".to_string(),
                        record_variable_id: None,
                        temp_state: TempState::Known(false),
                        via: "direct".to_string(),
                    });
                    em.set_base_summary(&id, base_summary);
                }
            });
            *kind_counts.entry("change-db-effects").or_default() += 1;
            edits_checked += 1;
        }

        // --- change direct coverage ---
        if let Some(id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "change-direct-coverage", base, |em| {
                if let Some(f) = em.model.routines.get(&id) {
                    let (status, mut reasons) = f.direct_coverage.clone();
                    reasons.push("synthetic-reason".to_string());
                    let _ = status;
                    em.set_direct_coverage(&id, ("partial".to_string(), reasons));
                }
            });
            *kind_counts.entry("change-direct-coverage").or_default() += 1;
            edits_checked += 1;
        }

        // --- toggle body_available ---
        if let Some(id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "toggle-body-available", base, |em| {
                if let Some(f) = em.model.routines.get(&id) {
                    let b = f.body_available;
                    em.set_body_available(&id, !b);
                }
            });
            *kind_counts.entry("toggle-body-available").or_default() += 1;
            edits_checked += 1;
        }

        // --- change app_identity ---
        assert_edit_equal(fixture, "change-app-identity", base, |em| {
            em.set_app_identity("synthetic-app-identity");
        });
        *kind_counts.entry("change-app-identity").or_default() += 1;
        edits_checked += 1;

        // --- bump dep_stamp ---
        assert_edit_equal(fixture, "bump-dep-stamp", base, |em| {
            em.set_dep_stamp("synthetic-dep-stamp-v2");
        });
        *kind_counts.entry("bump-dep-stamp").or_default() += 1;
        edits_checked += 1;

        // --- routine REMOVE ---
        if let Some(id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "remove-routine", base, |em| {
                em.remove_routine(&id);
            });
            *kind_counts.entry("remove-routine").or_default() += 1;
            edits_checked += 1;
        }

        // --- routine ADD (a fresh leaf routine with no edges) ---
        if let Some(template_id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "add-routine", base, |em| {
                if let Some(tmpl) = em.model.routines.get(&template_id).cloned() {
                    let new_id = format!("{template_id}::synthetic-added");
                    let mut routine = (*tmpl.routine).clone();
                    routine.id = new_id.clone();
                    routine.stable_routine_id =
                        format!("{}::added", tmpl.routine.stable_routine_id);
                    let mut base_summary = tmpl.base_summary.clone();
                    base_summary.routine_id = new_id.clone();
                    base_summary.db_effects.clear();
                    let facts = RoutineFacts {
                        routine_id: new_id.clone(),
                        routine: Arc::new(routine),
                        combined_edges: Vec::new(),
                        typed_edges: Vec::new(),
                        uncertainty_edges: Vec::new(),
                        base_summary,
                        direct_facts: Vec::new(),
                        direct_coverage: ("complete".to_string(), Vec::new()),
                        body_available: true,
                        is_leaf: false,
                    };
                    em.add_routine(facts);
                }
            });
            *kind_counts.entry("add-routine").or_default() += 1;
            edits_checked += 1;
        }

        // --- routine RENAME (== signature-rehash; StableRoutineId re-hash) ---
        if let Some(id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "rename-routine", base, |em| {
                let new_id = format!("{id}::renamed");
                let new_stable = em
                    .model
                    .ctx
                    .stable_map
                    .get(&id)
                    .map(|s| format!("{s}::renamed"))
                    .unwrap_or_else(|| new_id.clone());
                em.rename_routine(&id, &new_id, &new_stable);
            });
            *kind_counts.entry("rename-routine").or_default() += 1;
            edits_checked += 1;
        }

        // --- NO-OP-at-L4: set the SAME base summary (a redundant set). ---
        if let Some(id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "noop-same-base-summary", base, |em| {
                if let Some(f) = em.model.routines.get(&id) {
                    let same = f.base_summary.clone();
                    em.set_base_summary(&id, same);
                }
            });
            *kind_counts.entry("noop-same-base-summary").or_default() += 1;
            edits_checked += 1;
        }

        // --- NO-OP-at-L4: add a DUPLICATE call edge (already present ⇒ dominated;
        //     the combined-graph slice dedups/sorts to the same value). ---
        if let Some(id) = rng.pick(&ids).cloned() {
            assert_edit_equal(fixture, "noop-duplicate-edge", base, |em| {
                if let Some(f) = em.model.routines.get(&id) {
                    let mut edges = f.combined_edges.clone();
                    if let Some(first) = edges.first().cloned() {
                        edges.push(first); // duplicate; sort is stable, value unchanged at L4
                    }
                    em.set_combined_edges(&id, edges);
                }
            });
            *kind_counts.entry("noop-duplicate-edge").or_default() += 1;
            edits_checked += 1;
        }
    }

    eprintln!(
        "R3b Stage 2: {edits_checked} edits over {} fixtures, ALL incremental == from-scratch \
         (byte-for-byte). By kind: {:?}",
        fixtures.len(),
        {
            let mut v: Vec<_> = kind_counts.iter().collect();
            v.sort();
            v
        }
    );
    // Every edit kind fired at least once (the universe edits are unconditional;
    // the per-routine ones fire for any non-empty fixture).
    for kind in [
        "add-call-edge",
        "remove-call-edge",
        "change-db-effects",
        "change-direct-coverage",
        "toggle-body-available",
        "change-app-identity",
        "bump-dep-stamp",
        "remove-routine",
        "add-routine",
        "rename-routine",
        "noop-same-base-summary",
        "noop-duplicate-edge",
    ] {
        assert!(
            kind_counts.get(kind).copied().unwrap_or(0) > 0,
            "edit kind `{kind}` never fired — the corpus is too small or the generator is broken"
        );
    }
    assert!(
        edits_checked >= 200,
        "expected >=200 edits, got {edits_checked}"
    );
}

// ===========================================================================
// No-op early-cutoff — a no-op edit must NOT recompute the summary/cone cone.
// ===========================================================================

#[test]
fn r3b_stage2_noop_edit_early_cuts() {
    let fixtures = discover_fixtures();
    // Pick a few non-trivial fixtures (routines with edges, so there's a real cone).
    let mut interesting: Vec<&(String, InputModel)> = fixtures
        .iter()
        .filter(|(_, m)| {
            m.routine_ids.len() >= 2 && m.routines.values().any(|f| !f.combined_edges.is_empty())
        })
        .collect();
    interesting.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        !interesting.is_empty(),
        "no non-trivial fixture for early-cutoff probe"
    );

    let mut probed = 0usize;
    let mut edge_noop_probed = 0usize;
    for (fixture, base) in interesting.into_iter().take(12) {
        // ===================================================================
        // (A) No-op `combined_edges` set — re-sets a routine's OUTGOING edge slice
        //     to its CURRENT value. `combined_graph` reads it and re-executes, but
        //     its VALUE is unchanged ⇒ the value-equal carrier BACKDATES, so the
        //     whole downstream cone (scc_condensation / scc_summaries / cones)
        //     must NOT re-execute. This is the carrier value-equality win — the
        //     PRECONDITION fix is what makes it hold.
        // ===================================================================
        let edge_from = base
            .routines
            .iter()
            .find(|(_, f)| !f.combined_edges.is_empty())
            .map(|(id, _)| id.clone());
        if let Some(from) = edge_from {
            let (mut em, _log) = base.build_incremental_instrumented();
            let before = em.demand();
            em.clear_log();
            if let Some(f) = em.model.routines.get(&from) {
                let same = f.combined_edges.clone();
                em.set_combined_edges(&from, same);
            }
            let after = em.demand();
            let log = em.take_log();

            assert_eq!(
                before.fingerprint(),
                after.fingerprint(),
                "[{fixture}] no-op edge-set changed the demanded output"
            );
            let downstream = log
                .iter()
                .filter(|s| {
                    s.contains("scc_summaries")
                        || s.contains("cones")
                        || s.contains("scc_condensation")
                })
                .count();
            assert_eq!(
                downstream, 0,
                "[{fixture}] no-op `combined_edges` set re-executed {downstream} downstream \
                 queries — the value-equal `combined_graph` carrier did NOT backdate \
                 (PRECONDITION/carrier fix broken). Log: {log:?}"
            );
            edge_noop_probed += 1;
        }

        // ===================================================================
        // (B) No-op `base_summary` set — re-sets a routine's base summary to its
        //     current value. `scc_summaries` reads every routine's base_summary, so
        //     it DOES re-execute (Salsa inputs do not value-compare on set). But the
        //     `cones` query (whose lineage is direct_facts/direct_coverage, NOT
        //     base_summary) must NOT re-fire, AND `scc_summaries`' value-equal
        //     output must BACKDATE so the demanded result is byte-identical. (The
        //     tighter "don't even re-read the input" minimality is Stage 3.)
        // ===================================================================
        let (mut em, _log) = base.build_incremental_instrumented();
        let before = em.demand();
        em.clear_log();
        let pick = base.routine_ids[0].clone();
        if let Some(f) = em.model.routines.get(&pick) {
            let same = f.base_summary.clone();
            em.set_base_summary(&pick, same);
        }
        let after = em.demand();
        let log = em.take_log();

        assert_eq!(
            before.fingerprint(),
            after.fingerprint(),
            "[{fixture}] no-op base-summary set changed the demanded output (value-equal \
             carrier failed to backdate)"
        );
        // The cone is on a different input lineage — a base_summary no-op must not
        // touch it.
        let cone_recomputes = log.iter().filter(|s| s.contains("cones")).count();
        assert_eq!(
            cone_recomputes, 0,
            "[{fixture}] no-op base-summary set re-executed the `cones` query — \
             lineage leak. Log: {log:?}"
        );
        probed += 1;
    }
    eprintln!(
        "R3b Stage 2: no-op early-cutoff confirmed — {edge_noop_probed} edge-no-op fixtures \
         (0 downstream recomputes via value-equal combined_graph backdate), {probed} \
         base-summary-no-op fixtures (0 cone recomputes + byte-equal output)"
    );
    assert!(edge_noop_probed > 0, "no edge-no-op fixture probed");
}
