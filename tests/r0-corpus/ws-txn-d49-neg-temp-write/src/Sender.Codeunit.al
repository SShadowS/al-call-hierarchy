codeunit 50000 "D49 Sender"
{
    /// <summary>
    /// NEGATIVE: Insert() on a TEMPORARY record — no physical write transaction opened.
    /// Must produce ZERO d49 findings.
    /// </summary>
    procedure TempInsertThenMessage()
    var
        TempRec: Record "D49 Rec" temporary;
    begin
        TempRec.Init();
        TempRec."No." := 1;
        TempRec.Insert();
        Message('Done');
    end;
}
