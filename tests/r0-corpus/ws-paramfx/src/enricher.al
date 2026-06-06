codeunit 60101 "Line Enricher"
{
    procedure EnrichLine(var SalesLine: Record "Sales Line")
    begin
        if SalesLine.Amount > 0 then
            SalesLine.Amount := SalesLine.Amount;
    end;

    procedure AdjustLine(var SalesLine: Record "Sales Line")
    begin
        SalesLine.Validate(Amount);
        SalesLine.Reset();
    end;

    procedure NoRecordParam(n: Integer)
    begin
    end;
}
