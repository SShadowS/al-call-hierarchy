// Object-level (global) variable receivers — both directions must classify correctly:
//  - Codeunit-typed global receiver + record-op-named method → procedure call (traversable edge)
//  - Record-typed global receiver → record DB op (no regression)
codeunit 50104 "Probe Global Receivers"
{
	var
		GlobalArchive: Codeunit "Probe Insert File Archive";
		GlobalRec: Record "Probe File Archive";

	procedure CallViaGlobalCodeunit(InStr: InStream; AccNo: Code[20])
	begin
		GlobalArchive.Insert(InStr, AccNo);   // codeunit procedure call via GLOBAL variable
	end;

	procedure InsertViaGlobalRecord()
	begin
		GlobalRec.Init();
		GlobalRec.Insert(true);   // record op via GLOBAL record variable — must stay a DB_INSERT
	end;
}
