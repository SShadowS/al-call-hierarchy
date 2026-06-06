codeunit 50310 NestedCommit
{
	procedure NestedDoWork(DoIt: Boolean)
	var
		Done: Boolean;
	begin
		if DoIt then
			repeat
				Commit();
				Done := true;
			until Done;
	end;
}
