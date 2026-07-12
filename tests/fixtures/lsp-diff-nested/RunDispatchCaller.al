// VariableReceiverResolved probe (CDO layer 4): a `var` Codeunit-typed
// LOCAL variable's `.Run()` call — CDO's exact "sendqueue" shape (a plain
// procedure calling a codeunit-typed variable's `Run()`, which dispatches
// to that codeunit's `OnRun` trigger). See RunDispatchTarget.al.
codeunit 50319 "Run Dispatch Caller"
{
    procedure SendQueue()
    var
        QueueMgt: Codeunit "Run Dispatch Target";
    begin
        QueueMgt.Run();
    end;
}
