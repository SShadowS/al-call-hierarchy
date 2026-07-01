// beyond-1B.3b Task 6 fixture (d) — NEGATIVE: cross-app ambiguity. "Amb
// Table" is declared as a Table in BOTH dependency apps (CodeunitRecLibA and
// CodeunitRecLibB, see .alpackages/) — neither is this workspace's own app,
// so `resolve_object_ref` must DECLINE (`Ambiguous`), never guess one of the
// two. `TableNo` IS declared, so the implicit Rec stays `Record{table:
// None}` (a Record entity exists, its table just failed to resolve) — the
// non-builtin `Rec.Baz()` stays honest `Unknown`. Picking either dependency's
// table would be the cardinal sin (a false `Source` edge).
codeunit 50974 "Ambiguous Codeunit"
{
    TableNo = "Amb Table";

    trigger OnRun()
    begin
        Rec.Baz();
    end;
}
