// CU 50200: Has a local procedure that is ONLY reached via a codeunit-variable
// member call from Caller (CU 50201). Before resolver-upgrade (Spec 2 Task 7),
// member calls did not resolve so LocalHelper appeared dead. After Task 7 the
// member call resolves → LocalHelper is reachable → D14 must NOT flag it.
codeunit 50200 "MCR Helper"
{
	// local procedure — only callable from within this object... but al-sem's
	// resolver does not filter by access modifier when resolving member calls.
	// The call graph edge from Caller.CallHelper → MCR Helper.LocalHelper is
	// what makes LocalHelper reachable.
	local procedure LocalHelper()
	begin
	end;

	// Public procedure so the codeunit has at least one root (keeps it from being
	// entirely dead itself), but it does NOT call LocalHelper.
	procedure PublicEntry()
	begin
	end;
}
