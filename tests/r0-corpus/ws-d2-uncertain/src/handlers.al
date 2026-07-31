// ONE interface with exactly ONE implementing codeunit — one implementation,
// not two, keeps the resolved edge set deterministic.
interface "D2U IAlpha"
{
    procedure RunAlpha();
}

codeunit 64203 "D2U Alpha Impl" implements "D2U IAlpha"
{
    procedure RunAlpha()
    begin
    end;
}
