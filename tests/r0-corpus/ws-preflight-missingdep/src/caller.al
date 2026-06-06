/// <summary>
/// Codeunit that calls a procedure from a dependency app that is NOT provided.
/// This produces unresolved callsites and an opaque app in the coverage record.
/// Used by the preflight integration test.
/// </summary>
codeunit 63200 "PF Missing Dep Caller"
{
    procedure DoWork()
    begin
        // Call to a procedure in the missing dependency — will be unresolved
        MissingDepHelper();
    end;

    procedure DoMoreWork()
    begin
        // Another call to the missing dep — more unresolved callsites
        MissingDepUtil();
    end;
}
