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

    // NOT FLAGGED: the clone reassigns the table's PRIMARY KEY field before the
    // write — the write then targets a DIFFERENT physical row, so the clone is
    // functionally required (not the redundant re-write the rule targets).
    procedure CloneAndRemapPrimaryKey()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        if Cust.FindSet() then
            repeat
                CustCopy := Cust;
                CustCopy."No." := Cust."No." + '-COPY';
                CustCopy.Modify();
            until Cust.Next() = 0;
    end;

    // NOT FLAGGED: the clone reassigns the SOURCE cursor's CURRENT KEY field (set
    // via SetCurrentKey, not the table's declared PK) before the write — the
    // real-world MoveEmailLog shape (Continia's `EmailLog2 := EmailLog;
    // EmailLog2."Record ID" := ...; EmailLog2.Modify()` inside a loop, keyed on
    // SetCurrentKey's "Record ID", not the PK).
    procedure CloneAndRemapCurrentKey()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        Cust.SetCurrentKey(Name);
        if Cust.FindSet() then
            repeat
                CustCopy := Cust;
                CustCopy.Name := Cust.Name + '-COPY';
                CustCopy.Modify();
            until Cust.Next() = 0;
    end;
}
