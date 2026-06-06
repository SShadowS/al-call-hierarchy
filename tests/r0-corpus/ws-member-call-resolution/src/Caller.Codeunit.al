// CU 50201: Uses a codeunit-typed global variable to call MCR Helper.LocalHelper().
// This is the only callsite for LocalHelper — the variable-typed-call edge makes
// it reachable from OnRun (a root trigger).
codeunit 50201 "MCR Caller"
{
	var
		Helper: Codeunit "MCR Helper";

	trigger OnRun()
	begin
		Helper.LocalHelper(); // member call → resolves to MCR Helper.LocalHelper
	end;
}
