page 50308 "Nested Trigger Page"
{
    PageType = Card;
    SourceTable = "Nested Trigger Table";

    actions
    {
        area(Processing)
        {
            action(DoIt)
            {
                ApplicationArea = All;

                trigger OnAction()
                begin
                    DoSomething();
                end;
            }
        }
    }

    procedure DoSomething()
    begin
    end;
}
