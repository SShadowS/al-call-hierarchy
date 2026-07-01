// Case (i) valid extension → base protected (PageExtension sub-case, the
// gpt/gemini round-1 "generalize across extension kinds" requirement).
//
// AL semantics: legal, same reasoning as BarExtI.TableExt.al — `protected`
// is visible to a PageExtension of the declaring Page. Expected: COMPILES.
//
// IMPORTANT SCOPE NOTE: this task (`object_has_visible_member_candidate` /
// `ResolveIndex::object_extends`) is scoped to the `Record{table}` receiver
// path (`resolve_in_table_scope`), which is Table/TableExtension-only by
// construction (`ResolveIndex::table_extensions_of` indexes `TableExtension`
// only — a Page member call never reaches this helper). This fixture proves
// the IDENTITY relationship (`object_extends(BasePageExtI, BasePage)` is
// `true` — see the direct unit test
// `object_extends_generalizes_to_pageextension_base_page` in
// `src/program/resolve/resolver.rs`), demonstrating `object_extends` is
// GENERALIZED and ready for reuse, NOT that a bare call like `P()` inside
// `BasePageExtI` is ALREADY access-filtered end-to-end today — that would
// route through `resolve_bare`'s Step 2 ("Extension base"), a SEPARATE,
// pre-existing, currently access-UNFILTERED code path this task's brief
// explicitly does not touch (sole caller constraint: `resolve_in_table_
// scope`). Flagged as a follow-up in the task report.
page 52680 "BasePage"
{
    protected procedure P()
    begin
    end;
}
