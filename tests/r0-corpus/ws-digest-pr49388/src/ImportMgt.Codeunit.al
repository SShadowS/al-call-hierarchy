codeunit 50300 "Import Mgt"
{
	var
		FileArchive: Codeunit "File Archive";

	procedure ImportStream()
	begin
		FileArchive.InsertFile();
	end;
}
