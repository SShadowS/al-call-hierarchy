// beyond-1B.3b Task 3 fixture (e), NEGATIVE — builtin collision: declares a
// procedure named `StrLen`, arity 1 — same name+arity as the AL global
// intrinsic `StrLen(Text): Integer`. A bare `StrLen(x)` call from a Page
// sourced at THIS table must NOT assume the table wins; the collision is an
// UNPROVEN precedence (fail-closed to `Unknown`, never `Catalog`).
table 50978 "IR Builtin Collide Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure StrLen(FieldName: Text): Integer
    begin
        exit(0);
    end;
}
