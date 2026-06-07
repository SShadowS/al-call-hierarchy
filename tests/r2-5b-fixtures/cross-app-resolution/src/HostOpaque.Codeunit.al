codeunit 70002 "Host Opaque"
{
    procedure Split()
    var
        gone: Codeunit "Nowhere Cu";
    begin
        Codeunit.Run(Codeunit::"Absent Dep Cu");
        gone.M();
    end;
}