codeunit 50000 ExportMgr
{
    procedure RunExport(var Doc: Record "Sales Header")
    begin
        OnExport(Doc);
    end;

    [IntegrationEvent(false, false)]
    procedure OnExport(var Doc: Record "Sales Header")
    begin
    end;
}
