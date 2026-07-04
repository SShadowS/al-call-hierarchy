codeunit 50156 "PCR Target"
{
    procedure P(N: Integer): Integer
    begin
        exit(0);
    end;

    procedure P(S: Text): Integer
    begin
        exit(1);
    end;
}
