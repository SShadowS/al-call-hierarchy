codeunit 50150 "CH Target"
{
    procedure P(N: Integer): Integer
    begin
        exit(1);
    end;

    procedure P(S: Text): Integer
    begin
        exit(0);
    end;
}
