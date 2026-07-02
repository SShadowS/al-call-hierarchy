// Task 1 fixture (h) NEGATIVE (stranger-extension identity): this
// `TableExtension` textually says `extends "Dep Page"`, but a TableExtension
// can only ever resolve its base among TABLE-kind objects (kind-scoped
// lookup) — so it resolves to the WORKSPACE `StrangerTable.Table.al` (Table
// 60000 "Dep Page"), NEVER the ABI's Page 60000 "Dep Page" (a different
// `ObjectNodeId.kind`, even with the identical id AND name). The workspace
// stranger table declares zero procedures, so `P` stays genuinely absent here
// — this extension must NOT see the ABI base's `protected P()`.
tableextension 51002 "DepPageExtStranger" extends "Dep Page"
{
    procedure CallProtectedStranger()
    begin
        P();
    end;
}
