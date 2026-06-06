codeunit 51001 "Caller CU"
{
    procedure CallMember()
    var
        Other: Codeunit "Worker CU";
    begin
        Other.SomeMethod();
    end;

    procedure RunIt()
    begin
        Codeunit.Run(Codeunit::"Worker CU");
    end;

    procedure RunDynamic()
    var
        CUId: Integer;
    begin
        Codeunit.Run(CUId);
    end;
}
