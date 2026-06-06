codeunit 50300 "Item Manager"
{
    procedure UpdateItem(ItemNo: Code[20])
    var
        Item: Record Item;
    begin
        if Item.Get(ItemNo) then begin
            Item.Quantity += 1;
            Item.Modify();
        end;
    end;
}
