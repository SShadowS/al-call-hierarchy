codeunit 71000 "Chain Host"
{
    procedure UseChain()
    var
        cu: Codeunit "Dep Chain";
    begin
        cu.DoIt();
        cu.DoWrite();
    end;
}
