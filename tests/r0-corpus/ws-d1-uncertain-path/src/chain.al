// The d1 fixture whose WINNER cohort's representative path crosses uncertain
// nodes, so its finding carries a non-empty `confidence.evidence` in the r4
// golden. Every other d1 fixture's winner path is certain, which leaves
// `to_confidence`'s `is_empty()` fast path as the only thing any golden in the
// repository exercises.
//
// The chain is  Run (the loop) -> Enter -> Dispatch -> Touch (the db op):
//
//   * ONE db op in the whole workspace (`FindSet` in Touch) and ONE loop
//     reaching it, so there is exactly ONE terminal, ONE cohort and ONE winner
//     — the golden cannot go flaky on winner selection.
//   * The op is THREE hops from the loop, never inside it. A `Direct` winner
//     carries no uncertainty union at all (`build_cohort_rep`); only a `Reach`
//     winner walks its path, which is the code path under test.
//   * The union spans the seed ENTRY node (Enter) through the terminal, NOT the
//     loop's own routine — `path_uncertainty_ids` walks the reach chain's hops
//     and the terminal, and discards the seed, exactly as the `process_group`
//     oracle's `nodes_rev` does. Run is therefore deliberately NOT one of the
//     three nodes below.
//
// Each node's own list is already key-sorted (`uncertainties_by_node` stores
// `dedupe_uncertainties(...)`), so the ONLY way the concatenation can be out of
// order is for a LATER node to hold a key that sorts before an EARLIER node's.
// The interface-dispatch asymmetry (see `handlers.al`) produces exactly that:
//
//   node (path order)  uncertainties
//   -----------------  ------------------------------------------------------
//   Enter              opaque-callee@a, @b, @g
//                        — INHERITED from downstream; the three
//                          `interface-open-world`s are callsite-local and
//                          never reach it
//   Dispatch           interface-open-world@a, @b,
//                      opaque-callee@a, @b, @g
//   Touch (terminal)   interface-open-world@g, opaque-callee@g
//
// So along the path, seed-entry -> terminal:
//   discovery (first-seen) order  =  opaque-callee..., interface-open-world...
//   byte-sorted key order         =  interface-open-world..., opaque-callee...
// The two DIFFER ("i" < "o"), which is the one property no aggregate
// measurement — count, distinct-note set, byte total — can ever catch.
//
// The other three properties the golden pins:
//   * DE-DUPLICATION — `opaque-callee@a` appears at two nodes of the same path
//     (Enter and Dispatch), and `opaque-callee@g` at all three.
//   * RESOLUTION — six distinct entries, so an off-by-one in the id -> value
//     mapping shows up as wrong note text rather than a wrong count.
//   * THE TERMINAL ARM — `interface-open-world@g` reaches the union ONLY via
//     `path_uncertainty_ids`' terminal-node read (it is callsite-local to
//     Touch, so no hop node inherits it). Drop that read and the golden moves.
codeunit 63201 "D1U Driver"
{
    /// The seed: the loop. Its OWN uncertainties are deliberately outside the
    /// union (see the header) — it is here to make the winner a `Reach`.
    procedure Run()
    var
        Mid: Codeunit "D1U Mid";
        i: Integer;
    begin
        for i := 1 to 10 do
            Mid.Enter();
    end;
}

codeunit 63202 "D1U Mid"
{
    /// The seed ENTRY node — the first node of the union. Holds nothing of its
    /// own: everything it carries is INHERITED from further down the chain,
    /// which is what puts `opaque-callee` ahead of `interface-open-world` in
    /// path order.
    procedure Enter()
    var
        Step: Codeunit "D1U Step";
    begin
        Step.Dispatch();
    end;
}

codeunit 63207 "D1U Step"
{
    var
        Alpha: Interface "D1U IAlpha";
        Beta: Interface "D1U IBeta";

    /// The uncertain middle hop. No loop and no db op of its own — it owns two
    /// interface call sites and continues to the terminal.
    procedure Dispatch()
    var
        Sink: Codeunit "D1U Sink";
    begin
        Alpha.RunAlpha();
        Beta.RunBeta();
        Sink.Touch();
    end;
}

codeunit 63203 "D1U Sink"
{
    var
        Gamma: Interface "D1U IGamma";

    /// The terminal: one db op, no loop of its own. Its interface call site
    /// sits AFTER the db op so `FindSet` stays operation 0, and exists so the
    /// union's terminal-node read is covered too.
    procedure Touch()
    var
        Customer: Record "D1U Customer";
    begin
        Customer.FindSet();
        Gamma.RunGamma();
    end;
}
