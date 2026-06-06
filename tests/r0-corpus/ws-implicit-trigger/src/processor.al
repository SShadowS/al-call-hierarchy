codeunit 72101 "Order Processor"
{
    // ValidateQuantity is called from a loop — it calls Validate, which triggers
    // OnValidate via an implicit-trigger edge.  D1 will walk:
    //   ProcessLines (loop) -> callsite to ValidateQuantity
    //   -> implicit-trigger edge to OnValidate
    //   -> terminal FindSet
    // The intermediate hop note should include "(via implicit OnValidate trigger)".
    procedure ProcessLines()
    var
        i: Integer;
    begin
        for i := 1 to 10 do
            ValidateQuantity();
    end;

    local procedure ValidateQuantity()
    var
        Line: Record "Order Line";
    begin
        Line.Validate(Quantity);
    end;
}
