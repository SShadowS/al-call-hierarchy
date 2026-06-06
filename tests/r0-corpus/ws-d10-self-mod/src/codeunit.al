codeunit 50110 D10SelfMod
{
	procedure BadModify()
	var Customer: Record Customer;
	begin
		if Customer.FindSet() then
			repeat
				Customer."Last Date Modified" := Today;
				Customer.Modify();              // bad — modifying the iterating record
			until Customer.Next() = 0;
	end;

	procedure SafeModify()
	var Customer: Record Customer; Other: Record Customer;
	begin
		if Customer.FindSet() then
			repeat
				Other.Get(Customer."Buy-from No.");
				Other.Modify();                 // safe — different record var
			until Customer.Next() = 0;
	end;
}

table 18 Customer
{
	fields {
		field(1; "No."; Code[20]) { }
		field(50; "Last Date Modified"; Date) { }
		field(60; "Buy-from No."; Code[20]) { }
	}
}
