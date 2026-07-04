codeunit 50141 "CRN Target"
{
    procedure P(N: Integer): Integer
    begin
        exit(0);
    end;

    procedure P(S: Text): Integer
    begin
        exit(1);
    end;

    procedure Q(N: Integer): Boolean
    begin
        exit(true);
    end;

    procedure Q(S: Text): Boolean
    begin
        exit(false);
    end;

    procedure R(N: Integer): Boolean
    begin
        exit(true);
    end;

    procedure R(S: Text): Boolean
    begin
        exit(false);
    end;
}
