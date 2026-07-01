// The NAME target: a codeunit literally NAMED "80" (id 50100, unrelated to
// the numeric id 80). Declares `P()`, which RealById.Codeunit.al (id 80)
// deliberately lacks — proves the caller resolves by SHAPE (a quoted name
// reference), never by re-parsing the unquoted text "80" as a numeric id.
codeunit 50100 "80"
{
    procedure P()
    begin
    end;
}
