// App: PrimaryAppStranger, depends on DepAppStranger. StrangerTarget.Secret()
// is `internal` and DepAppStranger names NO friends -> stays honest Unknown
// (InternalNotVisible), same as pre-Task-1.5 behavior for this specific pair.
codeunit 53973 "StrangerCaller"
{
    procedure Trigger()
    begin
    end;
}
