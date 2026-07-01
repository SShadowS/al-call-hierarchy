// (b) Bare-call shadow: `Error('boom')` must resolve to the LOCAL `Error`
// procedure (Evidence::Source), not the `error` global intrinsic.
codeunit 50952 "ShadowCallerB"
{
    procedure CallB()
    begin
        Error('boom');
    end;

    procedure Error(Msg: Text)
    begin
    end;
}
