codeunit 50120 "MCD Target"
{
    procedure P(S: Text): Integer
    begin
        exit(0);
    end;

    procedure P(R: Record "MCD Rec"): Integer
    begin
        exit(1);
    end;
}
