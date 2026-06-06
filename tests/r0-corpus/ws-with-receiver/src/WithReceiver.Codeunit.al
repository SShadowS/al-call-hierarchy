codeunit 50108 "With Receiver"
{
    procedure DoWith()
    var
        R: Record "Probe Rec";
    begin
        with R do begin
            Insert();
            Modify;
        end;
    end;
}
