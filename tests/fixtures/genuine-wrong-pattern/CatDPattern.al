// Fixture demonstrating the Cat-D "different-object" genuine_wrong pattern.
//
// Scenario: Two codeunits define a procedure with the same name and same arity.
// A bare call from a third codeunit can ambiguously resolve to either target.
// When fresh picks CuA.SharedProc() but L3 picks CuB.SharedProc() (or vice
// versa), the targets are completely DISJOINT — neither is a subset of the
// other, and no interface implements relationship holds. This is a genuine_wrong
// site (Cat-D: different-object) that must be enumerated in the manifest and
// fixed by the disambiguation logic in 1B.3b.
//
// Note: this fixture does NOT contain a genuine_wrong in the fresh engine today
// (the bare call resolves unambiguously in the fixture because CuC only depends
// on CuA). It exists to document the SHAPE of the problem for future test
// coverage.

codeunit 50500 "CatDCuA"
{
    procedure SharedProc()
    begin
        // Implementation A
    end;
}

codeunit 50501 "CatDCuB"
{
    // Same name, same arity as CatDCuA.SharedProc.
    // In a real CDO workspace both codeunits are in scope from the caller,
    // creating the ambiguity that causes Cat-D genuine_wrong.
    procedure SharedProc()
    begin
        // Implementation B
    end;
}

codeunit 50502 "CatDCuC"
{
    procedure Caller()
    begin
        // Bare call: fresh resolves to CatDCuA.SharedProc (first-in-scope wins).
        // In CDO, if L3 used a different scope-ordering heuristic and picked
        // CatDCuB.SharedProc, this becomes a Cat-D genuine_wrong site.
        SharedProc();
    end;
}
