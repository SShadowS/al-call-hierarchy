codeunit 50301 EarlyExitGuarded
{
	procedure EarlyExitDoWork(var IsHandled: Boolean)
	begin
		if IsHandled then
			exit;
		Commit();
	end;
}
