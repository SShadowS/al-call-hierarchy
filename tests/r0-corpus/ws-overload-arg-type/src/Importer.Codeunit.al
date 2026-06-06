codeunit 50211 "Probe Ovl Importer"
{
    // arity-1 overload — irrelevant to the 2-arg callsites, present so the candidate
    // set is a real overload group.
    procedure ImportToFileArchive(Setup: Record "Probe Ovl Setup")
    begin
        Message('noop');
    end;

    // (Text, Record) overload — does NOT commit. If the resolver wrongly picked this
    // one (or stayed ambiguous), the transitive COMMIT below would never surface.
    procedure ImportToFileArchive(FileName: Text; Setup: Record "Probe Ovl Setup")
    var
        Archive: Record "Probe Ovl Archive";
    begin
        Archive.Init();
        Archive.Name := FileName;
        // intentionally no Insert / no Commit
    end;

    // (InStream, Record) overload — the only type-compatible match for the caller's
    // (InStream, Record) callsite, and the only path that reaches Commit().
    procedure ImportToFileArchive(InStr: InStream; Setup: Record "Probe Ovl Setup")
    var
        Archive: Record "Probe Ovl Archive";
    begin
        Archive.Init();
        Archive.Insert(true);
        Commit();
    end;
}
