codeunit 50101 D1Caller
{
    procedure DoStuff()
    var
        i: Integer;
    begin
        for i := 1 to 5 do
            Codeunit.Run(Codeunit::D1DepHelper);
    end;
}
