codeunit 50304 UnreachableCommit
{
	procedure UnreachableDoWork()
	begin
		Error('x');
		Commit();
	end;
}
