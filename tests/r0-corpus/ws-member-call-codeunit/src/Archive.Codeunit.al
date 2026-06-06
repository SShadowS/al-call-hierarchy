// The callee codeunit: its `Insert` procedure performs the REAL record insert and the
// Commit() that the digest must surface transitively on callers of `Archive.Insert`.
codeunit 50101 "Probe Insert File Archive"
{
	procedure Insert(InStr: InStream; AccNo: Code[20])
	var
		FileArchive: Record "Probe File Archive";
	begin
		FileArchive.Init();
		FileArchive.Insert(true);   // line 10: the real record insert (direct DB_INSERT here)
		Commit();                   // line 11: the COMMIT the digest must surface transitively
	end;
}
