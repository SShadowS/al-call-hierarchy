interface IProveProcessor
{
	procedure ProveProcess();
}

codeunit 50306 "Prove Processor A" implements IProveProcessor
{
	procedure ProveProcess()
	begin
		Commit();
	end;
}

codeunit 50307 "Prove Processor B" implements IProveProcessor
{
	procedure ProveProcess()
	begin
	end;
}

codeunit 50308 InterfaceCaller
{
	var
		Proc: Interface IProveProcessor;

	procedure CallProcess()
	begin
		Proc.ProveProcess();
	end;
}
