// Task 4 fixtures — chain-table extensions for the Xml framework family
// (`framework_returns.rs`) and the NEW RecordRef/FieldRef/KeyRef typed-return
// table (`recordref_returns.rs`), plus the HTTPCONTENT investigation
// regression pin. One PROCEDURE per scenario so `edges_for_object_routine`
// can isolate each call obligation cleanly, mirroring
// `tests/r0-corpus/ws-compound-framework/`'s layout (beyond-1B.3b Task 4).
// See PROOF.md for the real-CDO-source grounding of every positive fixture
// and the HTTPCONTENT investigation finding.
codeunit 51201 "CT Caller"
{
    // ---- POSITIVES (Xml framework chains) --------------------------------

    // (a1) `XmlElement.Create(Text)` [chain, arity 1] -> `Xml` -> `.AsXmlNode()`
    // [leaf]. Real CDO shape: `Codeunit 6175323 "CDO Xml Document"` /
    // `Codeunit 6175326 "CDO Xml Management"`, `XmlElement.Create(Name).
    // AsXmlNode()`.
    procedure TestXmlElementCreateArity1AsXmlNode()
    var
        NewNode: XmlNode;
    begin
        NewNode := XmlElement.Create('root').AsXmlNode();
    end;

    // (a2) `XmlElement.Create(Text, Text, Any)` [chain, arity 3] -> `Xml` ->
    // `.AsXmlNode()` [leaf]. Real CDO shape: `Codeunit 6175324 "CDO Xml
    // Node"`, `XmlElement.Create(Name, '', InnerText).AsXmlNode()`.
    procedure TestXmlElementCreateArity3AsXmlNode()
    var
        NewNode: XmlNode;
    begin
        NewNode := XmlElement.Create('root', '', 'InnerText').AsXmlNode();
    end;

    // (a3) `Node.AsXmlElement()` [chain] -> `.GetChildNodes()` [leaf]. Real
    // CDO shape: `Codeunit 6175324 "CDO Xml Node"`,
    // `Node.AsXmlElement().GetChildNodes()`.
    procedure TestXmlNodeAsXmlElementGetChildNodes()
    var
        Node: XmlNode;
        NodeList: XmlNodeList;
    begin
        NodeList := Node.AsXmlElement().GetChildNodes();
    end;

    // (a4) `Child.AsXmlText()` [chain] -> `.Value()` [leaf]. Real CDO shape
    // (property form there): `Codeunit 6175324 "CDO Xml Node"`,
    // `ChildNode.AsXmlText().Value := NewInnerText`. Exercised here in the
    // method-call form (also real AL syntax — confirmed elsewhere in CDO for
    // the sibling `FieldRef.Value()`, see PROOF.md) to prove the table entry
    // is form-correct (`is_method: true`).
    procedure TestXmlNodeAsXmlTextValue()
    var
        Child: XmlNode;
        V: Text;
    begin
        V := Child.AsXmlText().Value();
    end;

    // ---- POSITIVES (RecordRef-family chains) -----------------------------

    // (b) `RecRef.KeyIndex(1)` [chain, `RecordRef`->`KeyRef`] ->
    // `.FieldIndex(1)` [chain, `KeyRef`->`FieldRef`] -> `.Value()` [leaf].
    // Real CDO shape: `Codeunit 6175310 "CDO Subscribers"`,
    // `KeyRef.FieldIndex(1).Value`.
    procedure TestRecordRefKeyIndexFieldIndexValue()
    var
        RecRef: RecordRef;
        V: Variant;
    begin
        V := RecRef.KeyIndex(1).FieldIndex(1).Value();
    end;

    // (c) `RecRef.Field(1)` [chain, `RecordRef`->`FieldRef`] -> `.Caption()`
    // [leaf]. Covers the `Field` row of the table (the `~3 real sites`
    // exercise `FieldIndex`/`KeyIndex`, not `Field`, as a chain prefix — see
    // PROOF.md — this fixture proves the `Field` row independently).
    procedure TestRecordRefFieldCaption()
    var
        RecRef: RecordRef;
        Cap: Text;
    begin
        Cap := RecRef.Field(1).Caption();
    end;

    // ---- NEGATIVES (all must stay honest Unknown) ------------------------

    // (n1) Un-tabled Xml member used as an intermediate receiver:
    // `Attributes()` is a real XML catalog LEAF member (present in
    // `member_catalog.rs`'s `XML` set) but deliberately NOT chain-tabled for
    // this task — the outer `.Count()` call's receiver stays Unknown.
    procedure TestXmlUntabledMemberChain()
    var
        Node: XmlNode;
        Cnt: Integer;
    begin
        Cnt := Node.Attributes().Count();
    end;

    // (n2) Wrong FORM: a table METHOD-entry (`AsXmlElement()`, `is_method:
    // true`) invoked as a PROPERTY (no parens) — never matches.
    procedure TestXmlWrongFormPropertyInsteadOfMethod()
    var
        Node: XmlNode;
        NodeList: XmlNodeList;
    begin
        NodeList := Node.AsXmlElement.GetChildNodes();
    end;

    // (n3) Wrong ARITY: `XmlElement.Create()` (0 args) never matches — no
    // documented overload takes zero arguments.
    procedure TestXmlWrongArityCreate()
    var
        NewNode: XmlNode;
    begin
        NewNode := XmlElement.Create().AsXmlNode();
    end;

    // (n4) Wrong ARITY (RecordRef family): `KeyIndex(1, 2)` (2 args) never
    // matches the table's arity-1 entry.
    procedure TestRecordRefFamilyWrongArity()
    var
        RecRef: RecordRef;
        Cnt: Integer;
    begin
        Cnt := RecRef.KeyIndex(1, 2).FieldCount();
    end;

    // (n5) Same-named member on a NON-RecordRef-family receiver: `Rec:
    // Record "CT Item"` types `Record{..}`, not `RecordRef`/`FieldRef`/
    // `KeyRef` — the recordref-family table lookup never engages, even
    // though `"fieldindex"` happens to be a valid RecordRef/KeyRef table
    // member name.
    procedure TestRecordFieldIndexNotRecordRefFamily()
    var
        Rec: Record "CT Item";
        V: Variant;
    begin
        V := Rec.FieldIndex(1).Value();
    end;

    // (n6) `FieldRef.Value` chain-decline (round-1 I4, explicitly required by
    // the task brief): `Value` is variant-like LEAF data, never a chainable
    // receiver — a chained `.SomeMethod()` off it must stay Unknown.
    procedure TestFieldRefValueChainDecline()
    var
        SourceRecRef: RecordRef;
    begin
        SourceRecRef.Field(1).Value().SomeMethod();
    end;

    // (n7) Unvalidated/omitted entry stays declined: `FieldRef.Record()` is
    // a real, MS-Learn-documented method (returns `RecordRef`) but
    // deliberately out of this task's reviewed scope (round-1 I4's
    // enumerated handle set) — must stay Unknown until a future task adds
    // and validates it.
    procedure TestFieldRefRecordUnvalidatedDecline()
    var
        FRef: FieldRef;
        Num: Integer;
    begin
        Num := FRef.Record().Number();
    end;

    // (n8) HTTPCONTENT investigation finding (see PROOF.md): `Content.
    // AsText()` on a genuinely `HttpContent`-typed (platform value type)
    // receiver stays Unknown. `AsText`/`AsBlob`/`AsInStream`/`AsJson*` are
    // NOT real `HttpContent` members — verified against BOTH
    // methods-auto/httpcontent (live, fetched 2026-07-02) AND this project's
    // own SymbolReference-generated `member_builtins.json`; the catalog
    // (`Clear`/`GetHeaders`/`IsSecretContent`/`ReadAs`/`WriteFrom`) is
    // already complete and correct, not stale. The real methods with those
    // names belong to the UNRELATED System Application `Codeunit "Http
    // Content"` (`System.RestClient`), whose one real CDO call site
    // (`Codeunit 6175364 "CDO Universign E-Seal Service"`,
    // `Response.GetContent().AsText()`) was ALREADY resolved by the prior
    // plan v2.1 Task 3 cross-object-chain fix — see this harness's own
    // `cdo_full_program_coverage_and_self_reported_metric` ceiling-comment
    // history. This fixture regression-pins that the FRAMEWORK catalog is
    // NOT extended with a fabricated entry.
    procedure TestHttpContentAsTextStaysUnknown()
    var
        Content: HttpContent;
        T: Text;
    begin
        T := Content.AsText();
    end;
}
