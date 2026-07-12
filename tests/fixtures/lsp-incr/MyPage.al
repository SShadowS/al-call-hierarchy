page 50104 "LSP Incr Page"
{
    SourceTable = "LSP Incr Table";

    layout
    {
        area(Content)
        {
            repeater(Group)
            {
                field("No."; Rec."No.")
                {
                }
            }
        }
    }

    trigger OnOpenPage()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
    end;
}
