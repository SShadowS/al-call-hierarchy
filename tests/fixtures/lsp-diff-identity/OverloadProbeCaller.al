// Calls ONLY the 0-arg "Overload Probe Table".Compute() overload — see
// OverloadProbeTable.al's doc.
codeunit 50314 "Overload Probe Caller"
{
    procedure CallIt(): Integer
    var
        T: Record "Overload Probe Table";
    begin
        exit(T.Compute());
    end;
}
