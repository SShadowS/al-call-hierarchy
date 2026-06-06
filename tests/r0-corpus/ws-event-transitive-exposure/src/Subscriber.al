codeunit 50001 ExportLog
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::ExportMgr, 'OnExport', '', false, false)]
    local procedure LogExport(var Doc: Record "Sales Header")
    var
        L: Record "Document Log";
    begin
        L.Init();
        L."Document No." := Doc."No.";
        L.Insert(true);
    end;
}
