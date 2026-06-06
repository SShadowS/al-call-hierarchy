codeunit 50104 D4RepeatedGet
{
	procedure BadLookup()
	var SalesLine: Record "Sales Line"; Customer: Record Customer;
	begin
		if SalesLine.FindSet() then
			repeat
				Customer.Get('CUST001');
				Customer.Get('CUST001');
			until SalesLine.Next() = 0;
	end;

	procedure SafeLookup()
	var SalesLine: Record "Sales Line"; Customer: Record Customer;
	begin
		if SalesLine.FindSet() then
			repeat
				Customer.Get(SalesLine."Sell-to Customer No.");
			until SalesLine.Next() = 0;
	end;
}

table 18 Customer { fields { field(1; "No."; Code[20]) { } } }
table 37 "Sales Line"
{
	fields {
		field(1; "Document No."; Code[20]) { }
		field(2; "Sell-to Customer No."; Code[20]) { }
	}
}
