codeunit 50002 SalesAudit
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::SalesNotifier, 'OnNotifyComplete', '', false, false)]
    local procedure HAudit(var S: Record Sales)
    var
        L: Record AuditLog;
    begin
        L."Entry No." := 1;
        L.Insert(true);
    end;
}
