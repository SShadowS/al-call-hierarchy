// Record-field chains plan Task 4 — base table for the BARE implicit-Rec
// quoted-field receiver arm (`infer_receiver_type`'s Step 3a,
// `src/program/resolve/receiver.rs`): `"Field".X()` with NO `Rec.` prefix at
// all, written inside the TABLE's OWN procedure, means exactly
// `Rec."Field".X()`. Field names/types mirror real CDO grounding (the
// measured ~38-site UntrackedReceiver population): `Table 6175301 "CDO
// File"` field(20; "File Blob"; Blob), accessed bare from within the table's
// own procedures — see PROOF.md.
table 51520 "RBF Base"
{
    fields
    {
        field(1; "No."; Code[20]) { }

        // (a) POSITIVE: Blob field, bare (implicit Rec) — "File Blob".
        // CreateInStream(S) inside this table's own procedure must type
        // Framework(Blob) and resolve the Blob catalog leaf.
        field(2; "File Blob"; Blob) { }

        // Extra POSITIVE: a Text field — the measured "~1 Text[250] .Trim()"
        // population from the plan's grounding.
        field(3; "Some Text"; Text[250]) { }

        // NEGATIVE target: declared BUT also named identically to a
        // procedure declared below ("Shadowed Field") — round-2 soundness
        // correction (routine-shadow guard): AL's parens are optional on a
        // zero-argument call, so a bare quoted name is ambiguous between
        // this field and a parens-less call to the same-named procedure.
        // NOTE: real AL likely rejects a field/procedure name collision at
        // compile time (no `alc` available in this task's execution
        // environment to confirm either way — see PROOF.md); tree-sitter
        // does not validate cross-declaration semantic uniqueness, so this
        // fixture exercises OUR OWN fail-closed routine-shadow guard
        // directly, mirroring the established convention for the sibling
        // "Dup Field" duplicate-field fixture in `ws-record-field-chain`.
        field(4; "Shadowed Field"; Blob) { }
    }
    keys
    {
        key(PK; "No.") { Clustered = true; }
    }

    // The colliding procedure for "Shadowed Field" above.
    procedure "Shadowed Field"()
    begin
    end;

    // ---- POSITIVES -----------------------------------------------------

    // (a) `"File Blob".CreateInStream(S)` — bare implicit-Rec Blob field.
    // Real CDO shape: `Table 6175301 "CDO File"`,
    // `"File Blob".CreateInStream(ReadStream);` inside the table's own code.
    procedure TestBareBlobField()
    var
        S: InStream;
    begin
        "File Blob".CreateInStream(S);
    end;

    // Extra positive: a bare Text field — `.Trim()` is a real Text catalog
    // member.
    procedure TestBareTextField()
    var
        Result: Text[250];
    begin
        Result := "Some Text".Trim();
    end;

    // ---- NEGATIVES (all must stay honest Unknown) -----------------------

    // (f) Unknown quoted name: "No Such Field" is not declared anywhere on
    // "RBF Base" or its extensions.
    procedure TestBareUnknownField()
    begin
        "No Such Field".DoIt();
    end;

    // (c)+(d) QUOTE-PARITY + PRECEDENCE: a LOCAL var declared with the SAME
    // quoted name as a real field ("File Blob" is a genuine Blob field,
    // declared above) — the var must win outright (AL scoping: vars always
    // shadow fields), resolving as `Framework(Text)` (`.Trim()`), never
    // `Framework(Blob)`. Pre-quote-parity-fix, the raw quote-retaining
    // receiver text never matched the unquoted `VarDecl` name at all, so
    // this fixture would have fallen through to Step 3a and (post-Task-4)
    // resolved the FIELD instead — exactly the false-`Source` class this
    // fix exists to prevent.
    procedure TestBareVarShadowsFieldQuoteParity()
    var
        "File Blob": Text[100];
        Result: Text[100];
    begin
        Result := "File Blob".Trim();
    end;

    // Round-2 soundness correction: a same-named ROUTINE ("Shadowed Field",
    // declared above) must block field-typing — AL's parens are optional on
    // a zero-argument call, so this bare quoted name is ambiguous between
    // the field and a parens-less call to the procedure; must decline.
    procedure TestBareRoutineShadowsField()
    var
        S: InStream;
    begin
        "Shadowed Field".CreateInStream(S);
    end;
}
