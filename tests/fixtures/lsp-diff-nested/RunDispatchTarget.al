// VariableReceiverResolved probe (CDO layer 4): the RUN-DISPATCH target.
// See RunDispatchCaller.al: a `var` Codeunit-typed variable's `.Run()` call
// dispatches to THIS codeunit's `OnRun` trigger — mirrors CDO's
// `.dependencies/cdo/.../cdoqueuemanagement.codeunit.al::cdo queue
// management.onrun: new caller=sendqueue` finding.
codeunit 50318 "Run Dispatch Target"
{
    trigger OnRun()
    begin
    end;
}
