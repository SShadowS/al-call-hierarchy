codeunit 50110 "Neg Target"
{
    // Variant vs Integer — an InStream arg eliminates Integer (stream vs numeric), Variant survives.
    procedure V(X: Variant): Integer
    begin
        exit(0);
    end;

    procedure V(X: Integer): Integer
    begin
        exit(1);
    end;

    // (Integer, Text) vs (Integer, Code[20]) — a Text arg2 is text-family-compatible with BOTH → ambiguous.
    procedure I(A: Integer; B: Text): Integer
    begin
        exit(0);
    end;

    procedure I(A: Integer; B: Code[20]): Integer
    begin
        exit(1);
    end;

    // Interface vs Text — a Codeunit arg excludes NEITHER (both object-ish/unknown) → ambiguous.
    procedure O(X: Interface "Neg ILog"): Integer
    begin
        exit(0);
    end;

    procedure O(X: Text): Integer
    begin
        exit(1);
    end;
}
