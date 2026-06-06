// Primary codeunit: calls cross-app DepCU via a typed variable member call.
// Dep.InternalMethod() → cross-app + internal → D13 flags it.
// Dep.PublicMethod()   → cross-app + public   → D13 must NOT flag it.
codeunit 50114 D13McPrimary
{
	var
		Dep: Codeunit DepCU;

	trigger OnRun()
	begin
		Dep.InternalMethod(); // cross-app internal via member call → D13 flags
		Dep.PublicMethod();   // cross-app public → D13 must NOT flag
	end;
}
