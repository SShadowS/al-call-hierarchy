codeunit 50300 "Item Manager"
{
    procedure UpdateItem(ItemNo: Code[20])
    var
        Item: Record Item;
    begin
        Item."No." := ItemNo;
        Item.Modify();
    end;
}
