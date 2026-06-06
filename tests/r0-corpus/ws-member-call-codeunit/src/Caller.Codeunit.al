// DevPlan.md reproduction: a member call on a Codeunit-typed LOCAL variable whose
// procedure name collides with a record built-in (`Insert`). This is a procedure
// invocation, NOT a record DB operation — the classifier must consult the receiver's
// declared type, not the method name.
codeunit 50100 "Probe Caller"
{
	procedure RunImport(InStr: InStream; AccNo: Code[20])
	var
		Archive: Codeunit "Probe Insert File Archive";
	begin
		Archive.Insert(InStr, AccNo);   // codeunit procedure call, NOT a record insert
	end;
}
