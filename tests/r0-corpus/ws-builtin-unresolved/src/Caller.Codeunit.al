codeunit 50100 "BU Caller"
{
    procedure Run(L: List of [Text])
    var
        S: Text;
        N: Integer;
    begin
        S := CopyStr('abcdef', 1, MaxStrLen(S));
        N := StrLen(S);
        if N = 0 then
            Error('empty');
        L.Add(S);
    end;
}
