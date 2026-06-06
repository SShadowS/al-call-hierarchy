codeunit 50301 "File Archive"
{
	procedure InsertFile()
	var
		Rec: Record "File Archive Entry";
	begin
		Rec.Insert();    // line 7: DB_INSERT fact
		Commit();        // line 8: COMMIT fact — the roadmap's PR49388 anchor
	end;

	procedure ValidateAndError(OK: Boolean)
	begin
		if not OK then
			Error('Validation failed');   // line 13: ERROR_THROW direct
	end;
}
