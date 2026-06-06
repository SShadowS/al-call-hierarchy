codeunit 50303 "Import Dispatcher"
{
	var
		Processor: Interface IImportProcessor;

	procedure DispatchImport()
	begin
		Processor.Process();   // interface dispatch — open-world / polymorphic unresolved
	end;
}
