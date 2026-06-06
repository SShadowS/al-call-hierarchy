codeunit 50000 PostingMgr
{
    procedure DoPostA(var Rec: Record PostingEntry)
    var
        IsHandled: Boolean;
    begin
        IsHandled := false;
        OnBeforePost(Rec, IsHandled);
        if not IsHandled then
            Rec.Insert(true);
    end;

    procedure DoPostB(var Rec: Record PostingEntry)
    var
        IsHandled: Boolean;
    begin
        IsHandled := false;
        OnBeforePost(Rec, IsHandled);
        if not IsHandled then
            Rec.Modify(true);
    end;

    [IntegrationEvent(false, false)]
    procedure OnBeforePost(var Rec: Record PostingEntry; var IsHandled: Boolean)
    begin
    end;
}
