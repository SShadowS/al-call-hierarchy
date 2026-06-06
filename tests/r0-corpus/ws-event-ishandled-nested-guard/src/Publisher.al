codeunit 50000 PostingMgr
{
    procedure DoPost(var Rec: Record PostingEntry; ExtraCheck: Boolean)
    var
        IsHandled: Boolean;
    begin
        IsHandled := false;
        OnBeforePost(Rec, IsHandled);
        if ExtraCheck and not IsHandled then
            Rec.Insert(true);
    end;

    [IntegrationEvent(false, false)]
    procedure OnBeforePost(var Rec: Record PostingEntry; var IsHandled: Boolean)
    begin
    end;
}
