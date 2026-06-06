codeunit 50111 "Neg Caller"
{
    procedure CallVariant(S: InStream)
    var
        T: Codeunit "Neg Target";
    begin
        T.V(S);
    end;

    procedure CallIndistinct(A: Integer; B: Text)
    var
        T: Codeunit "Neg Target";
    begin
        T.I(A, B);
    end;

    procedure CallObject(L: Codeunit "Neg Target")
    var
        T: Codeunit "Neg Target";
    begin
        T.O(L);
    end;
}
