//! Phase 0: span-based site matcher fixture matrix (no env needed).
//!
//! Exercises [`match_sites`] — the cascade-resistance spine of the dual-run
//! differential harness.  All tests construct synthetic edges via
//! [`canonical_call_edge_for_test`] so no real workspace is required.

use al_call_hierarchy::program::resolve::differential::{
    SiteMatch, canonical_call_edge_for_test, match_sites,
};

// ---------------------------------------------------------------------------
// Test 1 (from brief): one missing L3 site must NOT cascade
// ---------------------------------------------------------------------------

/// Verifies the core cascade-resistance guarantee: when the L3 oracle is
/// missing exactly one site that the fresh side emits, that site becomes a
/// single `FreshOnly` and all other pairings are undisturbed.
#[test]
fn one_missing_site_does_not_cascade() {
    // Build 5 fresh sites at increasing spans; L3 has the same 5 minus the 2nd.
    let mk = |start: u32, fp: u64| canonical_call_edge_for_test("cu:c:run", start, fp);
    let fresh = vec![mk(10, 1), mk(20, 2), mk(30, 3), mk(40, 4), mk(50, 5)];
    let l3 = vec![mk(10, 1), mk(30, 3), mk(40, 4), mk(50, 5)];
    let matches = match_sites(&fresh, &l3);
    let paired = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Paired(_, _)))
        .count();
    let fresh_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::FreshOnly(_)))
        .count();
    let l3_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::L3Only(_)))
        .count();
    let unaligned = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Unaligned(_, _)))
        .count();
    // 4 clean pairs; the 2nd fresh site is the single FreshOnly; NO cascade on 3/4/5.
    assert_eq!(paired, 4, "matches: {matches:?}");
    assert_eq!(fresh_only, 1);
    assert_eq!(
        matches.len(),
        5,
        "every site must be in exactly one bucket: {matches:?}"
    );
    assert_eq!(l3_only, 0, "no L3-only sites in this test");
    assert_eq!(unaligned, 0, "no unaligned duplicates in this test");
}

// ---------------------------------------------------------------------------
// Test 2: duplicate calls on the same line pair cleanly
// ---------------------------------------------------------------------------

/// When two fresh sites and two L3 sites share the same strong key
/// `(unit, start_line, callee_fp)` (e.g. identical back-to-back calls on one
/// line), the matcher pairs them positionally — 2 `Paired`, no `Unaligned`.
#[test]
fn duplicate_calls_on_same_line_pair_cleanly() {
    let mk = |start: u32, fp: u64| canonical_call_edge_for_test("cu:c:run", start, fp);
    // Two identical sites in both fresh and L3.
    let fresh = vec![mk(10, 1), mk(10, 1)];
    let l3 = vec![mk(10, 1), mk(10, 1)];
    let matches = match_sites(&fresh, &l3);
    let paired = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Paired(_, _)))
        .count();
    let unaligned = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Unaligned(_, _)))
        .count();
    assert_eq!(paired, 2, "matches: {matches:?}");
    assert_eq!(
        unaligned, 0,
        "equal-count duplicates must not produce Unaligned"
    );
}

// ---------------------------------------------------------------------------
// Test 3: FreshOnly in a different (from,kind) group does not cascade
// ---------------------------------------------------------------------------

/// A fresh site whose caller has NO L3 peer at all (different `from` key →
/// different partition) is emitted as `FreshOnly`.  The two other sites from
/// the first caller still pair cleanly — proving that one partition's
/// mismatch is invisible to another partition.
#[test]
fn fresh_only_different_caller_does_not_cascade() {
    let mk = |caller: &str, start: u32, fp: u64| canonical_call_edge_for_test(caller, start, fp);
    let fresh = vec![
        mk("cu:c:run", 10, 1),
        mk("cu:c:run", 20, 2),
        mk("cu:c:post", 10, 1), // different caller — no L3 peer
    ];
    let l3 = vec![mk("cu:c:run", 10, 1), mk("cu:c:run", 20, 2)];
    let matches = match_sites(&fresh, &l3);
    let paired = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Paired(_, _)))
        .count();
    let fresh_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::FreshOnly(_)))
        .count();
    let l3_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::L3Only(_)))
        .count();
    // 2 clean pairs in "cu:c:run"; 1 FreshOnly in "cu:c:post"; no L3Only.
    assert_eq!(paired, 2, "matches: {matches:?}");
    assert_eq!(fresh_only, 1, "the cu:c:post site has no L3 peer");
    assert_eq!(l3_only, 0);
}
