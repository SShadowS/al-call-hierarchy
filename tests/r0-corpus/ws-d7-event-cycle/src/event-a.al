codeunit 50117 D7EventCycle
{
	[IntegrationEvent(false, false)]
	procedure OnA() begin end;

	procedure RaiseA() begin OnA(); end;

	[IntegrationEvent(false, false)]
	procedure OnB() begin end;

	procedure RaiseB() begin OnB(); end;

	[EventSubscriber(ObjectType::Codeunit, Codeunit::D7EventCycle, 'OnA', '', true, true)]
	local procedure SubA() begin RaiseB(); end;

	[EventSubscriber(ObjectType::Codeunit, Codeunit::D7EventCycle, 'OnB', '', true, true)]
	local procedure SubB() begin RaiseA(); end;
}
