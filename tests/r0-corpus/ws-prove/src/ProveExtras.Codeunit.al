table 50320 "Prove Entry"
{
	fields
	{
		field(1; "Entry No."; Integer) { }
		field(2; "Description"; Text[100]) { }
	}
	keys { key(PK; "Entry No.") { Clustered = true; } }
}

codeunit 50311 ProveTableWriter
{
	procedure WriteProveEntry()
	var
		Entry: Record "Prove Entry";
	begin
		Entry.Init();
		Entry."Entry No." := 1;
		Entry.Insert();
		Commit();
	end;
}

codeunit 50312 ProveEventPublisher
{
	procedure DoProvePost()
	begin
		OnBeforeProvePost();
	end;

	[IntegrationEvent(false, false)]
	procedure OnBeforeProvePost()
	begin
	end;
}

codeunit 50313 ProveUiRoutine
{
	procedure ShowProveMessage()
	begin
		Message('Hello');
	end;
}

codeunit 50314 ProveErrorRoutine
{
	procedure ValidateProve(OK: Boolean)
	begin
		if not OK then
			Error('Validation failed');
	end;
}
