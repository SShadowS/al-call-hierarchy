codeunit 50114 D14EntryPoints
{
	trigger OnRun()
	begin
		LiveHelper();
	end;

	local procedure LiveHelper() begin end;

	local procedure DeadHelper() begin end;
}
