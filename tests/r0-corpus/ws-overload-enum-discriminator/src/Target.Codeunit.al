codeunit 50140 "EN Target"
{
    procedure P(K: Enum "Probe Kind"): Integer
    begin
        Commit();
        exit(1);
    end;

    procedure P(S: InStream): Integer
    begin
        exit(0);
    end;
}
