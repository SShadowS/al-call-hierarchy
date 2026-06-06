codeunit 50120 "CE Target"
{
    procedure P(N: Integer): Integer
    var
        Rec: Record "CE Rec";
    begin
        Rec.Init();
        Rec.Insert(true);
        Commit();
        exit(1);
    end;

    procedure P(S: Text): Integer
    begin
        exit(0);
    end;
}
