codeunit 50100 D3TempCheck
{
    procedure Foo()
    var
        TempCustomer: Record Customer temporary;
    begin
        TempCustomer.FindFirst();
        if TempCustomer.Name <> '' then
            ;
    end;
}
