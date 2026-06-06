page 50400 "Item API"
{
    PageType = API;
    APIPublisher = 'rc';
    APIGroup = 'app';
    APIVersion = 'v1.0';
    EntityName = 'item';
    EntitySetName = 'items';
    SourceTable = Integer;

    layout
    {
        area(content)
        {
            repeater(Group)
            {
                field(Number; Rec.Number) { }
            }
        }
    }

    trigger OnOpenPage()
    begin
    end;

    procedure ApiHelper()
    begin
    end;
}
