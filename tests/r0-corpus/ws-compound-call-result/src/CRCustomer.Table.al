// beyond-1B.3b Task 3 fixture — target table for the `GetCustomer(): Record
// CRCustomer` positive (fixture a) and the `Update()`/PageInstance-intrinsic
// collision negative (fixture e, Rec/builtin-shadow).
table 51000 "CR Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { Clustered = true; }
    }

    // Non-builtin — the positive fixture (a) target member.
    procedure Name(): Text
    begin
    end;

    // Same name+arity as the bare-callable `PageInstance` intrinsic `Update`
    // (see `is_bare_builtin_or_page_intrinsic`) — used by fixture (e) to
    // exercise `resolve_bare`'s PROBE-THEN-DECIDE builtin-precedence collision
    // guard from inside a Page's implicit-Rec Step 3.
    procedure Update(): Record "CR Customer"
    begin
    end;
}
