codeunit 53970 "OverloadNTarget"
{
    procedure Foo(p: Integer)
    begin
    end;

    internal procedure Foo(p: Text)
    begin
    end;
}
