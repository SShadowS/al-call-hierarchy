codeunit 50210 "Probe Ovl Caller"
{
    // Calls the (InStream, Record) overload. By ARGUMENT TYPE this is unambiguous —
    // only the line-? (InStream, Record) overload of ImportToFileArchive fits — even
    // though name+arity alone sees two 2-arg candidates.
    procedure RunImport(InStr: InStream; Setup: Record "Probe Ovl Setup")
    var
        Importer: Codeunit "Probe Ovl Importer";
    begin
        Importer.ImportToFileArchive(InStr, Setup);
    end;

    // Genuinely un-disambiguable callsite: the first argument is a Variant, which is
    // type-compatible with BOTH overloads' first parameter (Text and InStream), so the
    // matcher cannot eliminate either candidate and it must STAY ambiguous — the fix must
    // not fabricate a resolution here. (A Variant is pinnable to type "Variant", but
    // typeRelation(Variant, _) is always "unknown", so no candidate is excluded.)
    procedure RunAmbiguous(Setup: Record "Probe Ovl Setup")
    var
        Importer: Codeunit "Probe Ovl Importer";
        Anything: Variant;
    begin
        Importer.ImportToFileArchive(Anything, Setup);
    end;
}
