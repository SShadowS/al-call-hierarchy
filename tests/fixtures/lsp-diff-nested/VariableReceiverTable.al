// VariableReceiverResolved probe (CDO layer 3): a table with two
// procedures, called from a CODEUNIT via a `var` PARAMETER receiver and a
// LOCAL variable receiver respectively. See VariableReceiverCaller.al.
table 50315 "Variable Receiver Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { }
    }

    procedure IsPdf(): Boolean
    begin
        exit(true);
    end;

    procedure IsPasswordProtected(): Boolean
    begin
        exit(false);
    end;
}
