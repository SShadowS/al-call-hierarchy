// Task 1 fixtures (a)/(b)/(d)/(f)/(g)/(i) — a workspace codeunit that is NOT
// an extension of the dep's "Dep Page"/"Dep Arity" objects, so it never
// satisfies the `Access::Protected` self-or-extends visibility rule. Every
// procedure below is one isolated call obligation (its own routine), so
// `edges_for_object_routine` can target each scenario independently.
codeunit 51000 "ProtCaller"
{
    var
        DepPage: Page "Dep Page";
        DepArity: Codeunit "Dep Arity";

    // (a) NEGATIVE (today: false route via SymbolOnly `candidates.first()`):
    // `P` is `protected` in the dep ABI; a non-extending Object-receiver call
    // must decline honest `Unknown(ProtectedNotVisible)`, never `Source`/`Abi`.
    procedure TestProtectedExcluded()
    begin
        DepPage.P();
    end;

    // (b) CONTROL: `Pub` carries no access modifier (`Access::Public`) — must
    // still resolve (Abi/Opaque boundary), proving the fix does not
    // over-decline a genuinely-visible ABI member.
    procedure TestPublicControl()
    begin
        DepPage.Pub();
    end;

    // (d) CONTROL: `internal`/`local` ABI routines are DROPPED entirely at
    // ingestion (unchanged by Task 1) — the name is genuinely absent, so
    // these stay `Unknown(MemberNotFound)`, never `ProtectedNotVisible`
    // (proving the local/internal drop is untouched by the protected-carry fix).
    procedure TestInternalAbsentControl()
    begin
        DepPage.I();
    end;

    procedure TestLocalAbsentControl()
    begin
        DepPage.L();
    end;

    // (f) mixed-arity mixed-access NEGATIVE: `GetWorker` is overloaded in the
    // dep ABI — arity-0 `protected` and arity-1 `public`. The arity-0 call
    // must go honest `Unknown`, NEVER silently select the visible arity-1
    // sibling (order/visibility-dependent selection is the false-`Source`
    // vector this task closes).
    procedure TestMixedArityProtectedArm()
    begin
        DepArity.GetWorker();
    end;

    // (f) mixed-arity POSITIVE control: the arity-1 `public` overload of the
    // SAME name resolves normally.
    procedure TestMixedArityPublicArm()
    begin
        DepArity.GetWorker(1);
    end;

    // (g)/(i): `Get(ID: Integer)` is the ONLY declared overload (public,
    // arity 1). Calling with arity 0 must NOT emit — exactly-one-same-name is
    // insufficient at the wrong arity (the existence boolean may be `true`,
    // but that is diagnostics-only, never edge evidence).
    procedure TestWrongArityPublicOnly()
    begin
        DepArity.Get();
    end;
}
