// Task 2 review fix (Finding 1) fixture: a table field named identically to
// a global variable declared on the caller object (`Caller.Codeunit.al`),
// but with a DIFFERENT type — the field a `with Rec do` block would rebind
// a same-named bare identifier to.
table 50961 "WS With Scope Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; SomeField; Decimal) { }
    }
}
