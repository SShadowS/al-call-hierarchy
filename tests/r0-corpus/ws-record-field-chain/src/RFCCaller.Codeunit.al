// Record-field chains plan Task 3 fixtures — `Rec."Field".X()` /
// `Rec.Field.X()` (`infer_compound_member_receiver`'s new non-method Member
// arm) + the EnumType-as-chain-base arm (`enum_chain_return_kind`). One
// PROCEDURE per scenario so `edges_for_object_routine` can isolate each call
// obligation cleanly, mirroring `tests/r0-corpus/ws-compound-framework/`'s
// layout. See PROOF.md for the real-CDO-source grounding of every positive
// fixture.
codeunit 51504 "RFC Caller"
{
    // ---- POSITIVES ---------------------------------------------------------

    // (a) `Rec."Error Message".CreateInStream(S)` — Blob field -> Framework
    // (Blob) -> CreateInStream is a real Blob catalog member. Real CDO shape:
    // `Table 6175273 "CDO E-Mail Log"`,
    // `Rec."Error Message".CreateInStream(IStream);`.
    procedure TestBlobFieldChain()
    var
        Rec: Record "RFC Base";
        S: InStream;
    begin
        Rec."Error Message".CreateInStream(S);
    end;

    // (b) `Rec."eSeal Service".Ordinals().Count()` — Enum field -> EnumType ->
    // `.Ordinals()` [NEW chain-base arm] -> Framework(List) -> `.Count()` is a
    // real List catalog member. The multi-level chain. Real CDO shape:
    // `Codeunit 6175455 "CDO E-Seal Setup Wizard"`,
    // `Rec."eSeal Service".Ordinals().Count() = 1`.
    procedure TestEnumFieldMultiLevelChain()
    var
        Rec: Record "RFC Base";
        N: Integer;
    begin
        N := Rec."eSeal Service".Ordinals().Count();
    end;

    // (c) `Rec."Ext Blob".CreateInStream(S)` — a TableExtension-declared field
    // (RFCBaseExt) resolves through the SAME arm as a base field (folding via
    // `ResolveIndex::field_in_table`, mirroring `table_extensions_of`).
    procedure TestExtensionFieldChain()
    var
        Rec: Record "RFC Base";
        S: InStream;
    begin
        Rec."Ext Blob".CreateInStream(S);
    end;

    // ---- NEGATIVES (all must stay honest Unknown) --------------------------

    // (e) Unknown field name: "No Such Field" is not declared anywhere on
    // "RFC Base" or its extensions — `field_in_table` genuinely finds
    // nothing, so the arm declines (falls through to Unknown).
    procedure TestUnknownFieldName()
    var
        Rec: Record "RFC Base";
    begin
        Rec."No Such Field".DoIt();
    end;

    // (f) Scalar-typed field: "Some Number" is `Integer` ->
    // `classify_type_text` -> `Primitive` -> `ReceiverType::Primitive` -> a
    // member call on it is an honest `CatalogMiss`, never a guessed route.
    procedure TestScalarFieldMemberCall()
    var
        Rec: Record "RFC Base";
    begin
        Rec."Some Number".DoIt();
    end;

    // (g) Duplicate field name across base + a VISIBLE extension: "Dup
    // Field" is declared by BOTH "RFC Base" (field 5, Blob) and "RFC Base
    // Ext" (field 51501, Text[50]) — `field_in_table` must decline
    // (fail-closed ambiguity), never arbitrarily pick one.
    procedure TestDuplicateFieldAcrossBaseExtension()
    var
        Rec: Record "RFC Base";
        S: InStream;
    begin
        Rec."Dup Field".CreateInStream(S);
    end;

    // (h) A Page (non-Record) receiver with a quoted member: `MyPage: Page
    // "RFC Page"` types `Object{kind: Page, ..}`, never `Record` — the
    // record-field arm's `Record{table: Some(..)}` guard must never engage,
    // even though `"Error Message"` coincidentally names a real Blob field
    // on the page's own SourceTable.
    procedure TestPageReceiverQuotedMember()
    var
        MyPage: Page "RFC Page";
        S: InStream;
    begin
        MyPage."Error Message".CreateInStream(S);
    end;

    // (i) A local variable named identically to a real field: `ErrCode:
    // Text[100]` (bare, no `Rec.` prefix — a DIFFERENT type than the real
    // `ErrCode` field, `Code[10]`) — AL scoping means the variable lookup
    // (Step 2, pre-existing, unaffected by Task 3) finds and types it FIRST;
    // the record-field arm is never even reached (it only fires for a
    // `Member{object, member}` whose OBJECT already resolved to a Record — a
    // bare identifier never takes that path). `.Trim()` is a real TEXT
    // catalog member, so this must resolve via Framework(Text), proving the
    // non-field (variable) binding wins and the field is never mis-typed.
    procedure TestVarNameShadowsFieldNameNonFieldWins()
    var
        ErrCode: Text[100];
    begin
        ErrCode.Trim();
    end;
}
