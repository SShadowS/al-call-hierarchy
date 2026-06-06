codeunit 50303 LoopCommit
{
	procedure LoopDoWork()
	var
		Done: Boolean;
	begin
		repeat
			Commit();
			Done := true;
		until Done;
	end;
}
