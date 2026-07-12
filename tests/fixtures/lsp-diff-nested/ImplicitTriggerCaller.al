codeunit 50312 "Implicit Trigger Caller"
{
    procedure DoInsert()
    var
        Rec: Record "Implicit Trigger Table";
    begin
        Rec.Insert(true);
    end;
}
