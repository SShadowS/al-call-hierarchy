codeunit 50112 D12DeadEvent
{
	[IntegrationEvent(false, false)]
	procedure OnAfterDoStuff(SomeArg: Integer) begin end;

	procedure DoStuff() begin
		OnAfterDoStuff(42);
	end;
}
