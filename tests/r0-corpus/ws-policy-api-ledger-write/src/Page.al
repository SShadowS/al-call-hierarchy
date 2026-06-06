page 50106 "Sales API"
{
    PageType = API;
    APIPublisher = 'test';
    APIGroup = 'test';
    APIVersion = 'v1.0';
    EntityName = 'sales';
    EntitySetName = 'sales';
    SourceTable = "Sales Ledger Entry";

    layout
    {
        area(Content)
        {
            repeater(Group) { field(entryNo; Rec."Entry No.") { } }
        }
    }

    actions
    {
        area(Processing)
        {
            action(WriteEntry)
            {
                trigger OnAction()
                begin
                    Rec."Entry No." := 1;
                    Rec.Insert(true);
                end;
            }
        }
    }
}
