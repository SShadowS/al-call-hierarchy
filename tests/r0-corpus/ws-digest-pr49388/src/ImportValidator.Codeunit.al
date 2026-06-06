codeunit 50302 "Import Validator"
{
	var
		Archive: Codeunit "File Archive";

	procedure ValidateImport(OK: Boolean)
	begin
		Archive.ValidateAndError(OK);   // calls through to Error() transitively
	end;
}
