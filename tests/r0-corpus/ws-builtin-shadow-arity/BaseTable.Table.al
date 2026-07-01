// beyond-1B.3b Task 1 review-fix regression fixture (Finding 1): the lookup
// precedence rewrite made a real, previously-undisclosed secondary behavior
// change — a base-table name match with the WRONG arity no longer
// short-circuits the Record arm; it now correctly falls through to a
// TableExtension that declares the matching arity. This table declares
// `Foo()` (arity 0) — a name match, but NOT an arity match for the arity-1
// call the caller fixture makes.
table 51000 "BaseTable"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure Foo()
    begin
    end;
}
