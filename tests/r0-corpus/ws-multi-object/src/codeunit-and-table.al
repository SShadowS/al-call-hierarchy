codeunit 50200 MultiObjectCaller
{
	procedure DoWork()
	var Customer: Record Customer;
	begin
		Customer.Get('X');
	end;
}

table 18 Customer
{
	fields { field(1; "No."; Code[20]) { } }
}
