codeunit 50200 Processor
{
    procedure Compute(Input: Integer): Integer
    var
        Result: Integer;
    begin
        Result := Input * 2;
        exit(Result);
    end;
}
