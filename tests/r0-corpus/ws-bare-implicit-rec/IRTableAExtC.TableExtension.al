// beyond-1B.3b Task 3 fixture (c), VISIBLE TABLEEXTENSION: extends
// "IR Table A" with a procedure the base table does NOT declare. A bare call
// to `ExtProc()` from a Page sourced at "IR Table A" must resolve through
// Step 3's `resolve_in_table_scope` (Task 2's visibility-scoped table ∪
// extensions search) to THIS extension's `ExtProc`.
tableextension 50973 "IR Table A Ext C" extends "IR Table A"
{
    procedure ExtProc(): Text
    begin
        exit('ext-c');
    end;
}
