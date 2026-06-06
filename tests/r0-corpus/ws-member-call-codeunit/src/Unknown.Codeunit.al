// Unknown-receiver case: `Mystery` is not declared anywhere (no local, parameter, or
// global). The classifier must NOT fabricate a record DB op — the callsite must surface
// as unresolved (unresolved-receiver-type on the ledger) so absence proofs are blocked.
codeunit 50103 "Probe Unknown Receiver"
{
	procedure CallOnUnknown()
	begin
		Mystery.Insert(true);   // undeclared receiver — never a DB op, never silent
	end;
}
