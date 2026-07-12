page 50309 "Nested Page Field Trigger"
{
    PageType = Card;
    SourceTable = "Nested Trigger Table";

    layout
    {
        area(Content)
        {
            field("No."; Rec."No.")
            {
                ApplicationArea = All;

                trigger OnLookup(var Text: Text): Boolean
                begin
                    HandleLookup();
                end;
            }
        }
    }

    procedure HandleLookup()
    begin
    end;
}
