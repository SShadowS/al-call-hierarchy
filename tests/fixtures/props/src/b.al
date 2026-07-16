codeunit 50003 Sc
{
    var
        GlobalNames: List of [Text];

    procedure P(ParamN: Integer)
    var
        LocalN: Integer;
    begin
        LocalN := ParamN;
        GlobalNames.Add('x');
    end;
}
