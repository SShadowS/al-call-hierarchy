// App: DepAppStranger. This app declares NO <InternalsVisibleTo> friends at
// all. CONTROL: proves the friend model doesn't over-grant — a true stranger
// caller must still be declined.
codeunit 53972 "StrangerTarget"
{
    internal procedure Secret()
    begin
    end;
}
