codeunit 50153 Dispatcher
{
    var
        Proc: Interface IProcessor;

    procedure Dispatch()
    begin
        Proc.Process();
    end;
}
