// Record-field chains plan Task 3 — base table for the `Rec."Field".X()`
// record-field chain arm (`infer_compound_member_receiver`,
// `src/program/resolve/receiver.rs`) and the EnumType-as-chain-base arm
// (`enum_chain_return_kind`, `src/program/resolve/framework_returns.rs`).
// Field names/types mirror real CDO grounding (grepped 2026-07-02):
// `Table 6175273 "CDO E-Mail Log"` field(101; "Error Message"; Blob),
// `Table 6175329 "CDO E-Seal Setup"` field(2; "eSeal Service"; Enum
// CDOESealService) — see PROOF.md.
table 51500 "RFC Base"
{
    fields
    {
        field(1; "No."; Code[20]) { }

        // (a) POSITIVE: Blob field — Rec."Error Message".CreateInStream(S)
        // must type Framework(Blob) and resolve the Blob catalog leaf.
        field(2; "Error Message"; Blob) { }

        // (b) POSITIVE: Enum field — the multi-level chain
        // Rec."eSeal Service".Ordinals().Count() must type EnumType, then
        // Framework(List) via the new enum_chain_return_kind arm, then
        // resolve List::count.
        field(3; "eSeal Service"; Enum "RFC Service") { }

        // (f) NEGATIVE: a scalar-typed field — a member call on it must
        // decline (classify_type_text("Integer") -> Primitive -> CatalogMiss).
        field(4; "Some Number"; Integer) { }

        // (g) NEGATIVE (paired with RFCBaseExt's own "Dup Field"): a field
        // name declared identically by BOTH the base table and a visible
        // TableExtension must decline (fail-closed ambiguity), never guess.
        field(5; "Dup Field"; Blob) { }

        // (i) NEGATIVE/regression-proof target: an UNQUOTED field name so a
        // same-named LOCAL VARIABLE (declared in the caller, a different
        // type) can be referenced BARE — proving Step 2's pre-existing
        // variable lookup wins outright and the record-field arm (which only
        // ever fires via a `Rec.`-qualified `Member` expression) never even
        // sees a bare identifier.
        field(6; ErrCode; Code[10]) { }
    }
    keys
    {
        key(PK; "No.") { Clustered = true; }
    }
}
