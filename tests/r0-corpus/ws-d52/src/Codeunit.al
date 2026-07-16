codeunit 50920 "D52 Demo"
{
    // FLAGGED (high): DeleteAll on a var record parameter — no temp proof, no filter.
    procedure NukeBuffer(var Buffer: Record "D52 Buffer")
    begin
        Buffer.DeleteAll();
    end;

    // FLAGGED (medium): ModifyAll variant.
    procedure StampAll(var Buffer: Record "D52 Buffer")
    begin
        Buffer.ModifyAll(Code, 'Y');
    end;

    // NOT FLAGGED: IsTemporary entry guard (G-2) proves tempness.
    procedure CleanupGuarded(var Buffer: Record "D52 Buffer")
    begin
        if not Buffer.IsTemporary() then
            Error('must be temporary');
        Buffer.DeleteAll();
    end;

    // NOT FLAGGED: parameter declared `temporary`.
    procedure CleanupDeclaredTemp(var TempBuffer: Record "D52 Buffer" temporary)
    begin
        TempBuffer.DeleteAll();
    end;

    // NOT FLAGGED: routine-local filter narrows the op (scoped cleanup).
    procedure CleanupFiltered(var Buffer: Record "D52 Buffer")
    begin
        Buffer.SetRange(Code, 'X');
        Buffer.DeleteAll();
    end;

    // NOT FLAGGED: local (non-parameter) receiver — that is d33's territory.
    procedure LocalDelete()
    var
        Buffer: Record "D52 Buffer";
    begin
        Buffer.DeleteAll();
    end;
}
