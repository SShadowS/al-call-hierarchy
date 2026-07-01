// beyond-1B.3b Task 1 fixture: a user-declared table procedure whose NAME+ARITY
// matches a genuine Record catalog builtin ("FieldNo", arity 1).  AL semantics:
// this local declaration SHADOWS the platform intrinsic for `Record Acme`
// receivers.  This is the exact shape behind the 42 real CDO
// `builtin-catalog-fp-collision` divergences (e.g. `Record::fieldno`,
// `Record::setrecfilter`).
table 50950 "Acme"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure FieldNo(FieldName: Text): Integer
    begin
        exit(0);
    end;
}
