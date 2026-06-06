codeunit 50111 D11NoGet
{
	procedure BadNoGet()
	var Customer: Record Customer;
	begin
		Customer."No." := 'X';
		Customer.Modify();              // bad — never loaded
	end;

	procedure SafeGet()
	var Customer: Record Customer;
	begin
		Customer.Get('X');
		Customer."Last Date Modified" := Today;
		Customer.Modify();              // safe
	end;

	procedure FromParam(var Customer: Record Customer)
	begin
		Customer."Last Date Modified" := Today;
		Customer.Modify();              // suppressed — caller is responsible
	end;
}

table 18 Customer
{
	fields {
		field(1; "No."; Code[20]) { }
		field(50; "Last Date Modified"; Date) { }
	}
}
