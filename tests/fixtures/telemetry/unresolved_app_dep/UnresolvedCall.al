codeunit 50100 "Test Caller"
{
    procedure RunTest()
    var
        ExternalCodeunit: Codeunit "Missing External Codeunit";
    begin
        ExternalCodeunit.PostInvoice('CUST001', 100);
    end;
}
