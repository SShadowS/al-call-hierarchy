// beyond-1B.3b Task 3 fixture (j), PRECEDENCE: PageExtension of
// "IR PageExt Base Page" (whose own `Foo` AND whose `SourceTable`'s `Foo`
// BOTH exist). A bare `Foo()` call from `CallFoo` (a procedure added by this
// PageExtension) must resolve to the BASE PAGE's `Foo` via Step 2
// (extension-base), NOT to `"IR PageExt Src Table"`'s `Foo` via Step 3
// (implicit-Rec) — Step 2 runs strictly BEFORE Step 3 in `resolve_bare`'s
// precedence order (pre-existing ordering; Task 3 does not change it, only
// pins it as a regression lock now that Step 3 is live).
pageextension 50989 "IR PageExt J" extends "IR PageExt Base Page"
{
    procedure CallFoo(): Text
    begin
        exit(Foo());
    end;
}
