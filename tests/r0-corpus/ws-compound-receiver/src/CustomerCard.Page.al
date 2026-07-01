// beyond-1B.3b Task 7 fixture — `CurrPage.<part>.Page` subpage-instance
// receivers (control-aware, fail-closed). See `infer_receiver_type`'s Step 0
// in `src/program/resolve/receiver.rs`.
//
// (a) POSITIVE: `CurrPage.Lines.Page.RefreshLines()` — the `Lines` Part
//     control's target is "Customer Card Part" (CustomerCardPart.Page.al),
//     which declares `RefreshLines()` — must resolve to it with
//     `Evidence::Source` and the exact target id.
// (b) NEGATIVE — control vs subpage: `CurrPage.Lines.Update(false)` /
//     `CurrPage.Lines.Editable(false)` (NO `.Page`) address the CONTROL
//     itself (structural methods), not the subpage INSTANCE — must stay
//     honest `Unknown`, never routed to "Customer Card Part".
// (c) NEGATIVE — deep chain: `CurrPage.Lines.Page.Foo.Bar()` has more than
//     one remaining segment after `Lines.Page` — stays `Unknown`.
// (d) NEGATIVE — unknown part: no control is named "Nope" — stays `Unknown`.
// (e) NEGATIVE — SystemPart/UserControl: `Notes` (systempart) and `MyAddIn`
//     (usercontrol) are NOT `Part` controls — even WITH a `.Page` accessor,
//     and also bare (no `.Page`), both must decline to `Unknown` rather than
//     fabricate a Page/Framework route.
page 50991 "Customer Card"
{
    PageType = Card;

    layout
    {
        area(Content)
        {
            part(Lines; "Customer Card Part")
            {
            }
            systempart(Notes; Notes)
            {
            }
            usercontrol(MyAddIn; "Microsoft.Dynamics.Nav.Client.BusinessChart")
            {
            }
        }
    }

    trigger OnOpenPage()
    begin
        // (a) POSITIVE
        CurrPage.Lines.Page.RefreshLines();

        // (b) NEGATIVE — control vs subpage
        CurrPage.Lines.Update(false);
        CurrPage.Lines.Editable(false);

        // (c) NEGATIVE — deep chain
        CurrPage.Lines.Page.Foo.Bar();

        // (d) NEGATIVE — unknown part
        CurrPage.Nope.Page.DoesNotMatter();

        // (e) NEGATIVE — SystemPart / UserControl, WITH `.Page`
        CurrPage.Notes.Page.ShowNotes();
        CurrPage.MyAddIn.Page.DoAddIn();

        // (e) NEGATIVE — SystemPart / UserControl, bare (no `.Page`)
        CurrPage.Notes.Refresh();
        CurrPage.MyAddIn.Trigger();
    end;
}
