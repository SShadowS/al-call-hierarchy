codeunit 50154 EmptyCaller
{
    var
        E: Interface IEmpty;

    procedure CallNothing()
    begin
        E.Nothing();
    end;
}
