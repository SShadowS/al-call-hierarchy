codeunit 50105 D5ModifyAll
{
	procedure BadLoop()
	var Customer: Record Customer;
	begin
		Customer.SetRange("Buy-from No.", '');
		if Customer.FindSet() then
			repeat
				Customer.Blocked := Customer.Blocked::All;
				Customer.Modify();
			until Customer.Next() = 0;
	end;

	procedure SafeLoop()
	var Customer: Record Customer; Helper: Record Customer;
	begin
		if Customer.FindSet() then
			repeat
				Helper.Get(Customer."Buy-from No.");
				Helper.Modify();
			until Customer.Next() = 0;
	end;
}

table 18 Customer
{
	fields {
		field(1; "No."; Code[20]) { }
		field(2; Blocked; Option) { OptionMembers = " ",All; }
		field(60; "Buy-from No."; Code[20]) { }
	}
}
