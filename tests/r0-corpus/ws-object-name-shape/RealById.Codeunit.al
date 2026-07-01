// The NUMERIC-id target: `Codeunit 80`. Deliberately does NOT declare `P()`
// — see Caller.Codeunit.al: a pre-fix run that (wrongly) resolves `Codeunit
// "80"` by numeric id 80 lands here and finds no `P`, producing a false
// `Unknown` instead of the correct `Source` edge to Named80.Codeunit.al.
codeunit 80 RealById
{
    procedure Other()
    begin
    end;
}
