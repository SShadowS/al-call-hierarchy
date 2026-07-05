page 50105 "Storage API"
{
    PageType = API;
    APIPublisher = 'test';
    APIGroup = 'test';
    APIVersion = 'v1.0';
    EntityName = 'storage';
    EntitySetName = 'storages';
    SourceTable = "Test Customer";

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
            action(WriteSecret)
            {
                trigger OnAction()
                begin
                    IsolatedStorage.Set('secret-key', 'secret-value', DataScope::Module);
                end;
            }
        }
    }
}
