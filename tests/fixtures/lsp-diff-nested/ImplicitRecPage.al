page 50314 "Implicit Rec Page"
{
    PageType = Card;
    SourceTable = "Implicit Rec Table";

    actions
    {
        area(Processing)
        {
            action(SetBackgroundPDFAction)
            {
                ApplicationArea = All;

                trigger OnAction()
                begin
                    SetBackgroundPDF();
                end;
            }
        }
    }

    trigger OnAfterGetCurrRecord()
    begin
        RefreshCache();
    end;
}
