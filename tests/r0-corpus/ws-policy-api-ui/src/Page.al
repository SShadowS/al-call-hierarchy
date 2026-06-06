page 50101 "Test Customer API"
{
    PageType = API;
    APIPublisher = 'test';
    APIGroup = 'test';
    APIVersion = 'v1.0';
    EntityName = 'testCustomer';
    EntitySetName = 'testCustomers';
    SourceTable = "Test Customer";
    DelayedInsert = true;

    layout
    {
        area(Content)
        {
            repeater(Group)
            {
                field(no; Rec."No.") { }
                field(name; Rec."Name") { }
            }
        }
    }

    actions
    {
        area(Processing)
        {
            action(DoAction)
            {
                trigger OnAction()
                begin
                    if Confirm('Confirm?') then;
                end;
            }
        }
    }
}
