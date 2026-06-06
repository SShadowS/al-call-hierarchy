table 50102 "Probe File Archive"
{
	fields
	{
		field(1; "Entry No."; Integer) { }
		field(2; "Bank Account No."; Code[20]) { }
	}
	keys
	{
		key(PK; "Entry No.") { Clustered = true; }
	}

	trigger OnInsert()
	begin
		// Implicit Rec receiver — must STAY classified as a record op after the
		// receiver-type-aware fix (Rec/xRec are always record receivers).
		Rec.TestField("Entry No.");
	end;
}
