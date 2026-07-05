codeunit 50104 "Install Mgr"
{
    Subtype = Install;

    trigger OnInstallAppPerCompany()
    var
        Cust: Record "Test Customer";
    begin
        Cust."No." := 'C001';
        Cust.Insert(true);
    end;
}
