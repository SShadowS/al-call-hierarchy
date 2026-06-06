codeunit 50309 TryFunctionCommit
{
	[TryFunction]
	procedure TryDoWork(): Boolean
	begin
		Commit();
	end;
}
