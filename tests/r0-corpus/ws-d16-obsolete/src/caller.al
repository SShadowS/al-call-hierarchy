codeunit 50116 D16Caller
{
	procedure UseOld()
	begin
		Codeunit.Run(Codeunit::D16OldWay);
	end;

	procedure UseVeryOld()
	begin
		Codeunit.Run(Codeunit::D16VeryOldWay);
	end;
}
