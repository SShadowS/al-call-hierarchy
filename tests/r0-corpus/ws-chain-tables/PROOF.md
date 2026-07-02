# Compiler proof — Xml + RecordRef-family chain tables, HTTPCONTENT investigation (Task 4)

**Status: SPEC-STATED, NOT COMPILER-RUN**, same caveat as `ws-compound-framework/PROOF.md`: no
AL compiler (`alc`/`ALC_EXE`) was available in this task's execution environment. The rows below
state the AL `RecordRef`/`FieldRef`/`KeyRef`/`XmlElement`/`XmlNode`/`XmlText`/`HttpContent`
methods-auto reference (Microsoft Learn, fetched 2026-07-02) rather than an actual `alc`
compile/diagnostic run, cross-checked against this project's own SymbolReference-generated
`tools/gen-al-builtins/out/member_builtins.json` (AL extension `ms-dynamics-smb.al-18.0.2293710`).
The `cdo_l3_semantic_audit_no_fresh_wrong` gate (`genuine_wrong == 0`) is the regression backstop;
this document is the intended CORRECTNESS proof artifact, reinforced by an EXHAUSTIVE grep against
the real CDO workspace (`U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`) for every fixture.

## What this fixture covers

1. Xml framework chains in `framework_return_kind` (`src/program/resolve/framework_returns.rs`):
   `XmlElement.Create(...)`, the full symmetric `AsXmlXxx()` conversion family, and
   `XmlElement.GetChildNodes()`.
2. A NEW typed-return table, `recordref_family_return_kind`
   (`src/program/resolve/recordref_returns.rs`), for the `RecordRef`/`FieldRef`/`KeyRef` handle
   family, plus the matching `ReceiverType::{RecordRef,FieldRef,KeyRef}` arm in
   `infer_compound_member_receiver` (`src/program/resolve/receiver.rs`).
3. A course-correction on the brief's third sub-item (HTTPCONTENT) — see "HTTPCONTENT
   investigation finding" below.
4. A genuine pre-existing bug found and fixed while grounding fixture (a1)/(a2)/(n3) against real
   CDO source — see "Step-4 bare-identifier guard fix" below.

## Real CDO source grounding (exhaustive grep, not a sample)

Every positive/negative fixture below is grounded in an ACTUAL call shape found by grepping the
real CDO workspace, not an invented scenario:

| Fixture | Real CDO site |
|---|---|
| (a1) `XmlElement.Create(Text).AsXmlNode()` | `Codeunit 6175323 "CDO Xml Document"` line 25: `XmlNewNode := XmlElement.Create(Name).AsXmlNode();`; `Codeunit 6175326 "CDO Xml Management"` line 30 (same shape) |
| (a2) `XmlElement.Create(Text,Text,Any).AsXmlNode()` | `Codeunit 6175324 "CDO Xml Node"` lines 120/131: `XmlElement.Create(Name, '', '').AsXmlNode()` / `XmlElement.Create(Name, '', InnerText).AsXmlNode()` |
| (a3) `Node.AsXmlElement().GetChildNodes()` | `Codeunit 6175324 "CDO Xml Node"` line 90: `NodeList := Node.AsXmlElement().GetChildNodes();` — the SAME `AsXmlElement()` chain entry also resolves 3 more real sites in the same file (lines 94/121/132/142: `....AsXmlElement().Add(...)`) |
| (a4) `Child.AsXmlText().Value()` | `Codeunit 6175324 "CDO Xml Node"` line 100 (property form there): `ChildNode.AsXmlText().Value := NewInnerText;` |
| (b) `RecRef.KeyIndex(1).FieldIndex(1).Value()` | `Codeunit 6175310 "CDO Subscribers"` line 1084: `DocNo := Format(KeyRef.FieldIndex(1).Value, 0, 2);` (KeyRef is a local var of type `KeyRef`); `Codeunit 6175399 "CDO Data Delete Handler"` line 217: `SourceFieldRef := SourceRecRef.KeyIndex(1).FieldIndex(1);` |
| — `RecordRef.KeyIndex(1)` chain alone | `Codeunit 6175399 "CDO Data Delete Handler"` line 212: `if SourceRecRef.KeyIndex(1).FieldCount = 0 then` — the SAME `KeyIndex` entry this fixture set exercises |
| (c) `RecRef.Field(1).Caption()` | No real CDO chain uses `.Field(n)` as an intermediate receiver (every real `.Field(n)` site assigns straight to a `FieldRef` var) — this fixture proves the `Field` row of the table independently, since it is validated (methods-auto/recordref) even though unexercised as a chain prefix in this corpus |
| (n8) `Content.AsText()` on `HttpContent` | See "HTTPCONTENT investigation finding" below |

This adds up to exactly the **~10 real Xml sites** and **~3 real RecordRef-family sites** the task
brief called for: 2 (`create` arity 1) + 2 (`create` arity 3) + 1 (`asxmlelement`→`getchildnodes`) +
4 (`asxmlelement`→`add`) + 1 (`asxmltext`→`value`) = 10 Xml sites; `KeyIndex`→`FieldCount`,
`KeyIndex`→`FieldIndex`, `KeyRef.FieldIndex`→`Value` = 3 RecordRef-family sites.

## HTTPCONTENT investigation finding — course correction

The brief's third sub-item asked to "extend HTTPCONTENT with the SymbolReference/MS-Learn-verified
methods (`AsText`, `AsBlob`, `AsInStream`, `AsJson*`, …) — SymbolReference-verified real methods"
because the catalog was allegedly stale. **This premise does not hold up under verification and was
NOT implemented as stated:**

- `methods-auto/httpcontent` (Microsoft Learn, fetched 2026-07-02, dated 2025-08-08, "Available or
  changed with runtime version 1.0" — i.e. current, not historical) lists exactly 5 `HttpContent`
  instance methods: `Clear()`, `GetHeaders(var HttpHeaders)`, `IsSecretContent()`,
  `ReadAs(var SecretText | var Text | var InStream)`, `WriteFrom(Text | SecretText | InStream)`.
  **No `AsText`/`AsBlob`/`AsInStream`/`AsJson*`.**
- This project's own SymbolReference-generated `tools/gen-al-builtins/out/member_builtins.json`
  (`ms-dynamics-smb.al-18.0.2293710`) independently lists the SAME 5 methods for `"HttpContent"` —
  a byte-for-byte match with `member_catalog.rs`'s existing `HTTPCONTENT` phf set. **The catalog is
  already complete and correct; it was never stale.**
- The methods named `AsText`/`AsBlob`/`AsInStream` DO exist, but on a completely UNRELATED type:
  the System Application's `Codeunit "Http Content"` (`System.RestClient` namespace,
  `learn.microsoft.com/.../application/system-application/codeunit/system.restclient.http-content`)
  — a source-level wrapper Codeunit, resolved via ordinary OBJECT/procedure resolution
  (`ReceiverType::Object`), never via the `Framework(HttpContent)` catalog. `classify_type_text`
  cleanly distinguishes the two: `Codeunit "Http Content"` (first token `"codeunit"`) vs.
  `HttpContent` (first token `"httpcontent"`) — there is no name-collision risk.
- The ONE real CDO site matching this shape, `Codeunit 6175364 "CDO Universign E-Seal Service"`,
  `ProcessSealResponse`, `Response.GetContent().AsText()` / `.AsBlob()` (where
  `Response: Codeunit "Http Response Message"`), was **already resolved by the prior plan v2.1 Task
  3 cross-object-chain fix** — see `tests/program_resolve_harness.rs`'s
  `cdo_full_program_coverage_and_self_reported_metric` ceiling-comment history (search
  `"Response.GetContent().AsText()"`), which documents the exact adjudication (System Application
  id 2356/2354, confirmed against the System App's embedded source directly).

Per this project's fail-closed cardinal rule and "uncertain OMITTED" policy (`framework_returns.rs`
module doc), adding `AsText`/`AsBlob`/`AsInStream`/`AsJson*` to the `HttpContent` FRAMEWORK catalog
would have been a **fabricated entry that cannot ever fire correctly** (no real AL receiver typed as
the platform `HttpContent` value type has those members) — the opposite of this task's mandate.
Fixture (n8) instead REGRESSION-PINS the correct behavior: `Content.AsText()` on a genuinely
`HttpContent`-typed receiver stays honest `Unknown`.

## Step-4 bare-identifier guard fix (discovered while grounding this fixture)

While building fixture (a1)/(a2) against real CDO source, a genuine PRE-EXISTING fail-open bug was
found in `infer_receiver_type`'s Step 4 (`src/program/resolve/receiver.rs`): `classify_type_text`
was called on the RAW receiver text unconditionally, and its `Xml` arm uses a prefix wildcard
(`s.starts_with("xml")`, the ONLY non-exact-match arm in the whole function — every other framework
type requires an EXACT string match). For a COMPOUND receiver whose FULL text happens to start with
`"xml"` (e.g. the outer `.AsXmlNode()` call's receiver text in
`XmlElement.Create('root').AsXmlNode()`, which is the WHOLE inner expression
`"xmlelement.create('root')"`), Step 4 would short-circuit straight to `Framework(Xml)` BEFORE
Steps 5/6's real per-hop chain-typing ever ran — meaning an UNTABLED or WRONG-ARITY Xml chain
(confirmed with the deliberately-untabled 0-arg `XmlElement.Create()`, fixture n3) would incorrectly
resolve instead of declining. Fixed by gating Step 4 to genuine bare identifiers only
(`!receiver_lc.contains('.') && !receiver_lc.contains('(')`), matching the step's own documented
intent ("bare identifier" — see the module doc). Verified: reverting this fix locally reproduces
the false-positive on fixture (n3) (`XmlElement.Create().AsXmlNode()` resolving to `Catalog` instead
of `Unknown`); the `ws-compound-framework` fixture (beyond-1B.3b Task 4) never exercised this path
because none of its base names happen to start with the one wildcarded framework-keyword prefix.

## Case-by-case matrix

| Case | Fixture routine | Rust test(s) |
|---|---|---|
| (a1) POSITIVE: `XmlElement.Create(Text)` chain, arity 1 | `TestXmlElementCreateArity1AsXmlNode` | `ws_chain_tables_xml_create_arity1_as_xml_node_resolves_catalog` |
| (a2) POSITIVE: `XmlElement.Create(Text,Text,Any)` chain, arity 3 (real CDO arity) | `TestXmlElementCreateArity3AsXmlNode` | `ws_chain_tables_xml_create_arity3_as_xml_node_resolves_catalog` |
| (a3) POSITIVE: `AsXmlElement()` chain -> `GetChildNodes()` leaf | `TestXmlNodeAsXmlElementGetChildNodes` | `ws_chain_tables_xml_as_xml_element_get_child_nodes_resolves_catalog` |
| (a4) POSITIVE: `AsXmlText()` chain -> `Value()` leaf | `TestXmlNodeAsXmlTextValue` | `ws_chain_tables_xml_as_xml_text_value_resolves_catalog` |
| (b) POSITIVE: `KeyIndex`->`KeyRef`, `FieldIndex`->`FieldRef`, `Value` leaf | `TestRecordRefKeyIndexFieldIndexValue` | `ws_chain_tables_recordref_keyindex_fieldindex_value_resolves_catalog` |
| (c) POSITIVE: `Field`->`FieldRef`, `Caption` leaf | `TestRecordRefFieldCaption` | `ws_chain_tables_recordref_field_caption_resolves_catalog` |
| (n1) NEGATIVE: un-tabled Xml member (`Attributes`) | `TestXmlUntabledMemberChain` | `ws_chain_tables_xml_untabled_member_chain_stays_unknown` |
| (n2) NEGATIVE: wrong form (`AsXmlElement` no parens) | `TestXmlWrongFormPropertyInsteadOfMethod` | `ws_chain_tables_xml_wrong_form_property_instead_of_method_stays_unknown` |
| (n3) NEGATIVE: wrong arity (`Create()`, 0 args) | `TestXmlWrongArityCreate` | `ws_chain_tables_xml_wrong_arity_create_stays_unknown` |
| (n4) NEGATIVE: wrong arity (`KeyIndex(1,2)`) | `TestRecordRefFamilyWrongArity` | `ws_chain_tables_recordref_family_wrong_arity_stays_unknown` |
| (n5) NEGATIVE: same-named member, non-RecordRef-family receiver | `TestRecordFieldIndexNotRecordRefFamily` | `ws_chain_tables_record_fieldindex_not_recordref_family_stays_unknown` |
| (n6) NEGATIVE: `FieldRef.Value` chain-decline (round-1 I4) | `TestFieldRefValueChainDecline` | `ws_chain_tables_fieldref_value_chain_decline_stays_unknown` |
| (n7) NEGATIVE: unvalidated/omitted entry (`FieldRef.Record()`) | `TestFieldRefRecordUnvalidatedDecline` | `ws_chain_tables_fieldref_record_unvalidated_stays_unknown` |
| (n8) NEGATIVE: HTTPCONTENT investigation regression pin | `TestHttpContentAsTextStaysUnknown` | `ws_chain_tables_httpcontent_astext_stays_unknown` |
