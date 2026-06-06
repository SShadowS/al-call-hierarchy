codeunit 50700 "D21 Demo"
{
    // FLAGGED: TestField without prior Get/Find.
    procedure TestFieldWithoutLoad()
    var
        Customer: Record Customer;
    begin
        Customer.TestField(Name);
    end;

    // FLAGGED: CalcFields without prior load.
    procedure CalcFieldsWithoutLoad()
    var
        Customer: Record Customer;
    begin
        Customer.CalcFields("Balance (LCY)");
    end;

    // NOT FLAGGED: TestField after Get.
    procedure TestFieldAfterGet()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        Customer.TestField(Name);
    end;

    // NOT FLAGGED: by-var parameter — caller is responsible for loading.
    procedure WithParameter(var Customer: Record Customer)
    begin
        Customer.TestField(Name);
    end;
}
