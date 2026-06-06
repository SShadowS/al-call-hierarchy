codeunit 52000 "Hub CU"
{
    procedure FanOut()
    begin
        StepA();
        StepB();
    end;

    local procedure StepA()
    begin
    end;

    local procedure StepB()
    begin
    end;

    procedure ManyUnknowns()
    var
        First: Variant;
        Second: Variant;
    begin
        First.DoThing();
        Second.DoOther();
    end;
}
