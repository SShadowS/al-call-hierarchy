// plan v2.1 Task 3 fixtures — `Var.Method().X()` cross-object call-result
// chain resolution (see `infer_cross_object_chain_receiver`, `src/program/
// resolve/receiver.rs`). One procedure per scenario so `edges_for_object_
// routine`/`outer_member_route` can isolate each call obligation cleanly.
codeunit 51206 "CrossChainCaller"
{
    var
        Helper: Codeunit "CC Helper";
        IFoo: Interface ICCFoo;
        IBar: Interface ICCBar;
        Response: Codeunit "Dep Http Response";
        ArityChain: Codeunit "Dep Arity Chain";
        DepOverload: Codeunit "Dep Overload";
        DepCollapse: Codeunit "Dep Collapse";

    // (a) POSITIVE: SOURCE prefix. `GetCustomer(No)` (unique arity-1,
    // `Record "CC Customer"` return) types the chain receiver `Record{table:
    // Some(CCCustomer)}`; `Name` is a non-builtin Customer procedure — must
    // resolve `Source`, exact target id.
    procedure TestSourcePrefix()
    var
        No: Code[20];
    begin
        Helper.GetCustomer(No).Name();
    end;

    // (b) POSITIVE: ABI prefix carrying a nested `Subtype`. `GetContent()`'s
    // declared return `Codeunit "Dep Http Content"` (reconstructed from the
    // ABI `Subtype`, Task 2) types the chain receiver `Object{Codeunit, "dep
    // http content"}`; `ReadAs` is a PUBLIC ABI member on that object — must
    // resolve `Opaque`/`AbiSymbol`.
    procedure TestAbiPrefix()
    begin
        Response.GetContent().ReadAs();
    end;

    // (c) NEGATIVE — leaf visibility: `GetContent()` types the chain exactly
    // like (b), but the leaf `Secret` is an ABI `internal` member — never
    // visible to this non-friend caller app (dropped entirely at ABI
    // ingestion) — proves the new chain-typing arm does not bypass Phase B's
    // ordinary visibility discipline at the leaf.
    procedure TestAbiLeafInternalNotVisible()
    begin
        Response.GetContent().Secret();
    end;

    // (d) POSITIVE: single-implementer interface prefix. `ICCFoo` has
    // EXACTLY ONE implementer (`CC Foo Impl`) in the closure — `resolve_
    // member`'s Interface fan-out yields exactly 1 route, the route-count
    // guard accepts, and the chain types `Object{Codeunit, "cc helper"}`
    // (AL guarantees the implementer's signature matches the interface's);
    // `DoWork` must resolve `Source`.
    procedure TestInterfaceSingleImpl()
    begin
        IFoo.GetHelper().DoWork();
    end;

    // (N1) NEGATIVE — polymorphic prefix: `ICCBar` has TWO implementers —
    // `resolve_member`'s Interface fan-out yields 2 routes; the route-count
    // guard must decline (conservative, never a guessed pick).
    procedure TestInterfacePolymorphicDeclines()
    begin
        IBar.GetHelper().DoWork();
    end;

    // (N2a) NEGATIVE — builtin-only prefix: `Rec.Next()` resolves via the
    // platform Record catalog (`RouteTarget::Builtin`), which carries no
    // modeled return type to chain onto.
    procedure TestBuiltinPrefixDeclines()
    var
        Rec: Record "CC Customer";
    begin
        Rec.Next().Name();
    end;

    // (N2b) NEGATIVE — wrong-arity SOURCE prefix: `GetCustomer` is declared
    // ONLY at arity 1; called here with arity 0 — `resolve_member`'s Object
    // arm returns a single `Unresolved(OverloadAmbiguous)` route.
    procedure TestWrongAritySourcePrefixDeclines()
    begin
        Helper.GetCustomer().Name();
    end;

    // (N3) NEGATIVE — ABI same-name overloads with DIFFERENT returns:
    // `Dep Overload` declares two `Get` overloads at the SAME arity (1),
    // differing only in the parameter's object kind (`Codeunit`/`Page`);
    // ABI parameter types are degraded (no `Subtype` on parameters), but the
    // two overloads' OUTER kind still differs here, so they remain two
    // distinct arity-1 candidates — `resolve_member` cannot pick between
    // them and must decline, never guessing either return type.
    procedure TestAbiOverloadAmbiguousDeclines()
    var
        SomeCodeunit: Codeunit "CC Helper";
    begin
        DepOverload.Get(SomeCodeunit).Name();
    end;

    // (N4a) NEGATIVE — scalar return: `GetCount(): Integer` has nothing to
    // dispatch a member call on.
    procedure TestScalarReturnDeclines()
    begin
        Helper.GetCount().Name();
    end;

    // (N4b) NEGATIVE — no declared return type at all.
    procedure TestNoReturnTypeDeclines()
    begin
        Helper.DoNothing().Name();
    end;

    // (N5) NEGATIVE — cross-app-ambiguous return: `GetShared()`'s declared
    // return `Codeunit "Dep Shared"` names an object declared IDENTICALLY in
    // BOTH `CrossChainDep` and `CrossChainDep2` — genuinely ambiguous in this
    // workspace's dependency closure; `parsed_type_to_receiver` (and, at the
    // leaf, `resolve_member`'s own `graph.resolve_object` re-lookup) both
    // decline rather than guess either dependency's codeunit.
    procedure TestCrossAppAmbiguousReturnDeclines()
    begin
        Response.GetShared().Name();
    end;

    // (N6) NEGATIVE — Name+Id cross-validation mismatch (Task 2): `GetMismatch
    // ()`'s declared `Subtype` names "Dep Http Content" but carries the WRONG
    // `Id` (99999, not that object's real id 60101) — the resolved object's
    // `declared_id` disagrees with the Subtype's `Id`, so the whole receiver
    // typing declines rather than trust a name-only match.
    procedure TestNameIdMismatchDeclines()
    begin
        Response.GetMismatch().ReadAs();
    end;

    // (N7/N9) NEGATIVE — DEFERRED record-field/property chain: `Rec."No."`
    // (property/field-access form, NO parens) is never this arm — the arm is
    // STRICTLY the procedure-CALL form (round-1 I7). `"No."` is a genuine
    // field on "CC Customer", not a procedure, so this stays honestly
    // `Unknown` regardless.
    procedure TestFieldPropertyChainDeclines()
    var
        Rec: Record "CC Customer";
    begin
        Rec."No.".Name();
    end;

    // (N8) NEGATIVE — 3-level chain, middle hop fails to type: hop 1
    // (`Helper.GetCustomer(No)`) types fine (`Record{CCCustomer}`); hop 2
    // (`<hop1>.NoSuchMethod()`) has no such member on "CC Customer" (source
    // or catalog) — declines to `Unknown`; the OUTER `.Name()` call's
    // receiver is therefore `Unknown` too — no partial guessing propagates
    // through a failed middle hop.
    procedure TestThreeLevelMiddleHopFailsDeclines()
    var
        No: Code[20];
    begin
        Helper.GetCustomer(No).NoSuchMethod().Name();
    end;

    // (N10) NEGATIVE — wrong-arity ABI prefix: `Dep Arity Chain` declares
    // `Get(ID: Integer): Codeunit "Dep Http Content"` — ONE candidate, but
    // ONLY at arity 1; called here with arity 0 — a single visible same-name
    // ABI candidate at the WRONG arity must not emit.
    procedure TestWrongArityAbiDeclines()
    begin
        ArityChain.Get().ReadAs();
    end;

    // (N11) NEGATIVE — COLLAPSED ABI overload survivor (Task 3 review fix):
    // `Dep Collapse` declares two `Get` overloads at the SAME arity (1) AND
    // the SAME OUTER parameter kind (`Codeunit`), differing ONLY in the
    // parameter's Subtype (`Dep A` vs `Dep C`). `AbiParameter::type_text`
    // fingerprints only the outer type keyword — never a param's Subtype —
    // so both overloads hash to the IDENTICAL `RoutineNodeId` and collapse
    // to ONE arbitrary survivor at ABI ingestion (unlike (N3) above, where
    // the outer kind itself differs and the two overloads stay genuinely
    // distinct candidates). The two overloads' RETURN types also differ
    // (`Dep Http Content` vs `Dep Arity Chain`) — trusting the collapsed
    // survivor's `return_type` would risk typing the chain to the WRONG
    // object. Must decline (`RoutineNode::abi_overload_collapsed`).
    procedure TestAbiOverloadCollapsedDeclines()
    begin
        DepCollapse.Get(Helper).ReadAs();
    end;
}
