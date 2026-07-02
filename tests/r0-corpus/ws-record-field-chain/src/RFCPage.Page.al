// (h) NEGATIVE target: a Page-typed receiver. `MyPage: Page "RFC Page"`
// types `ReceiverType::Object{kind: Page, ..}`, never `Record` — the
// record-field arm's `Record{table: Some(..)}` guard must never engage for
// it, even though the quoted member text coincidentally matches a real
// field name on the page's own SourceTable.
page 51503 "RFC Page"
{
    PageType = Card;
    SourceTable = "RFC Base";

    layout
    {
        area(Content)
        {
            field("No."; Rec."No.") { ApplicationArea = All; }
        }
    }
}
