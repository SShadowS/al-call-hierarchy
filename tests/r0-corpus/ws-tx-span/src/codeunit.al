codeunit 50101 TxSpan
{
	procedure DoWork()
	var
		Customer: Record Customer;
	begin
		Customer.Get('A');
		Customer."Last Date Modified" := Today;
		Customer.Modify();
		Commit();
		Customer.Get('B');
		Customer.Modify();
	end;
}

table 18 Customer
{
	fields { field(1; "No."; Code[20]) { } field(50; "Last Date Modified"; Date) { } }
}
