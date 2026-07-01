// beyond-1B.3b Task 3 fixture (d), NEGATIVE — sibling-extension ambiguity: TWO
// visible TableExtensions of "IR Table A" (this file declares both — separate
// object ids, same base) each declare `procedure Dup()` with the SAME
// name+arity. `resolve_in_table_scope`'s cardinality rule (Task 2) counts 2
// visible candidates and returns the honest ambiguous `Unknown` — a bare
// `Dup();` call must NEVER pick one extension over the other.
tableextension 50975 "IR Table A Ext D1" extends "IR Table A"
{
    procedure Dup(): Text
    begin
        exit('d1');
    end;
}

tableextension 50976 "IR Table A Ext D2" extends "IR Table A"
{
    procedure Dup(): Text
    begin
        exit('d2');
    end;
}
