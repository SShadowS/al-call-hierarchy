page 50102 "Dispatcher API"
{
    PageType = API;
    APIPublisher = 'test';
    APIGroup = 'test';
    APIVersion = 'v1.0';
    EntityName = 'dispatcher';
    EntitySetName = 'dispatchers';
    SourceTable = "Dispatch Setup";

    layout
    {
        area(Content)
        {
            repeater(Group) { field(no; Rec."No.") { } }
        }
    }

    actions
    {
        area(Processing)
        {
            action(Dispatch)
            {
                trigger OnAction()
                var
                    S: Record "Dispatch Setup";
                begin
                    S.Get(Rec."No.");
                    Codeunit.Run(S."Codeunit Id");
                end;
            }
        }
    }
}
