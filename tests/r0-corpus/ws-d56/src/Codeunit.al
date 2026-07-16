codeunit 50926 "D56 Demo"
{
    // FLAGGED: clone of the loop cursor written back inside the loop.
    procedure CloneAndModify()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        if Cust.FindSet() then
            repeat
                CustCopy := Cust;
                CustCopy.Name := 'X';
                CustCopy.Modify();
            until Cust.Next() = 0;
    end;

    // NOT FLAGGED: cursor modified directly (d10's territory, not d56's).
    procedure DirectModify()
    var
        Cust: Record "D56 Customer";
    begin
        if Cust.FindSet() then
            repeat
                Cust.Name := 'X';
                Cust.Modify();
            until Cust.Next() = 0;
    end;

    // NOT FLAGGED: copy taken outside any loop.
    procedure CopyOutsideLoop()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        Cust.FindFirst();
        CustCopy := Cust;
        CustCopy.Modify();
    end;

    // NOT FLAGGED: clone inside the loop but never written back.
    procedure CloneReadOnly()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        if Cust.FindSet() then
            repeat
                CustCopy := Cust;
            until Cust.Next() = 0;
    end;

    // NOT FLAGGED: the SOURCE cursor is a temporary in-memory buffer being
    // materialized into a persisted record — a different row, and the copy is a
    // SQL-free struct copy (the DO false-positive shape: temp-buffer → persisted).
    procedure MaterializeBuffer()
    var
        TempBuf: Record "D56 Customer" temporary;
        Cust: Record "D56 Customer";
    begin
        if TempBuf.FindSet() then
            repeat
                Cust := TempBuf;
                Cust.Name := 'x';
                Cust.Modify();
            until TempBuf.Next() = 0;
    end;
}
