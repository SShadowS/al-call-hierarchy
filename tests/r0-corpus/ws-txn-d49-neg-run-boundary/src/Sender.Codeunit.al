codeunit 50000 "D49 Sender"
{
    /// <summary>
    /// NEGATIVE: Modify() then Codeunit.Run() (checked) then Message().
    /// The Run boundary suppresses refutation-grade — must produce ZERO d49 findings
    /// (sound under disputed Q0 semantics — conservative approximation).
    /// </summary>
    procedure ModifyRunMessage()
    var
        Rec: Record "D49 Rec";
        CU: Codeunit "D49 Helper CU";
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        if CU.Run(Rec) then;
        Message('Done');
    end;
}

codeunit 50001 "D49 Helper CU"
{
    TableNo = "D49 Rec";

    trigger OnRun()
    begin
        // intentionally empty
    end;
}
