codeunit 50130 "FD Target"
{
    procedure P(N: Integer): Integer
    begin
        Commit();
        exit(1);
    end;

    procedure P(S: Text): Integer
    begin
        exit(0);
    end;
}
