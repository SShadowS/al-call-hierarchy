page 50300 "D14 Page With Helper"
{
    layout
    {
        area(content)
        {
            field("Test"; Rec.Name) { }
        }
    }

    // No-call-site procedure on a Page — al-sem doesn't yet model property-expression
    // references like `Caption = MyCaption()`, so we cannot prove this unreachable.
    // D14 must NOT flag it.
    local procedure ComputeCaption(): Text
    begin
        exit('hi');
    end;

    var
        Rec: Record Integer;
}

codeunit 50301 "D14 Page Helper Codeunit"
{
    // Local procedure on a Codeunit with no call site → must be flagged (codeunit call
    // graph is fully modelled).
    local procedure DeadCodeunitLocal()
    begin
    end;

    trigger OnRun()
    begin
    end;
}
