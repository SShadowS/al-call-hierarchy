page 50311 "Shared Name Two"
{
    PageType = Card;

    // Genuinely never called by anyone — see SharedNameTwo.al's doc.
    // Legacy's collapsed identity slot wrongly credits it with the
    // codeunit's real caller and stays silent on the diagnostics axis.
    procedure DoSomething()
    begin
    end;
}
