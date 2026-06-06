codeunit 50302 ConditionalCommit
{
	procedure ConditionalDoWork(DoIt: Boolean)
	begin
		if DoIt then
			Commit();
	end;
}
