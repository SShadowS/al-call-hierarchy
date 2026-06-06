codeunit 50101 "XA Worker"
{
    // P: only does Error('x') — always-error callee, no normal return path.
    procedure P()
    begin
        Error('x');
    end;
}
