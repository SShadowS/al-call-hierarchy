// beyond-1B.3b Task 3 fixture (j) support: the BASE page for
// `IRPageExtJ.PageExtension.al`. `SourceTable = "IR PageExt Src Table"` (which
// ALSO declares a `Foo`), but this page ALSO declares its OWN `Foo` — the
// target `resolve_bare`'s Step 2 (extension base) must resolve a bare `Foo()`
// call in the PageExtension to, ahead of Step 3's implicit-Rec.
page 50988 "IR PageExt Base Page"
{
    SourceTable = "IR PageExt Src Table";

    layout
    {
        area(Content)
        {
        }
    }

    procedure Foo(): Text
    begin
        exit('page-foo');
    end;
}
