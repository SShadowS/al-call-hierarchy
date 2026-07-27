// Three interfaces, each with exactly ONE implementing codeunit in the workspace.
//
// A RESOLVED interface dispatch is what makes the middle of the d1 chain
// uncertain, and it produces TWO uncertainties at the SAME call site:
//   * `interface-open-world` (the uncertainty edge — `combined_graph.rs`), which
//     is CALLSITE-LOCAL: `is_callsite_local_kind` keeps it on the routine that
//     owns the call site and never propagates it to a caller;
//   * `opaque-callee` (`db_effect_solver.rs`, from the `interface`-kind edge),
//     which DOES propagate up the call graph.
// That asymmetry is what makes an EARLIER node on the path carry a strict
// subset of a LATER node's uncertainties — see `chain.al`.
//
// One implementation each (not two) keeps the resolved edge set, and therefore
// the cohort, deterministic.
interface "D1U IAlpha"
{
    procedure RunAlpha();
}

interface "D1U IBeta"
{
    procedure RunBeta();
}

interface "D1U IGamma"
{
    procedure RunGamma();
}

codeunit 63204 "D1U Alpha Impl" implements "D1U IAlpha"
{
    procedure RunAlpha()
    begin
    end;
}

codeunit 63205 "D1U Beta Impl" implements "D1U IBeta"
{
    procedure RunBeta()
    begin
    end;
}

codeunit 63206 "D1U Gamma Impl" implements "D1U IGamma"
{
    procedure RunGamma()
    begin
    end;
}
