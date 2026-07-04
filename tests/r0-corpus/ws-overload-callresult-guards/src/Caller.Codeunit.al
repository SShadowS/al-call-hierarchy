// T3 (pageext-merge-and-final-residual plan) call-result guard fixtures:
// (a) is a NEGATIVE (must stay `AmbiguousResolved`); (b)/(c) are POSITIVE
// proofs that the passive builtin-return catalog NEVER fires for a name a
// source procedure shadows — proven by the shadowing procedure's OWN
// (different) return type driving a confident pick, not the catalog's.
codeunit 50140 "CRN Caller"
{
    // (a) The inner-uniqueness decline: `Ambiguous` has TWO arity-1
    // overloads (a genuine, compiler-legal AL overload pair). This
    // increment's `type_call_result_arg_bare` re-queries `resolve_bare` with
    // NO argument evidence (module doc: "no recursion into pick_candidate")
    // — a same-arity same-object overload set is therefore ALWAYS seen as
    // ambiguous from the inner call-result-typing path, regardless of what
    // the inner call's own real argument would otherwise have picked. The
    // arg position degrades to untyped, degrading the WHOLE outer call.
    procedure RunInnerOverloadAmbiguous()
    var
        T: Codeunit "CRN Target";
    begin
        T.P(Ambiguous(5));
    end;

    local procedure Ambiguous(X: Integer): Integer
    begin
        exit(X);
    end;

    local procedure Ambiguous(X: Text): Integer
    begin
        exit(0);
    end;

    // (b) SHADOWED-NAME fixture (mandatory, plan v2.1 addenda): a SOURCE
    // procedure named `Format` with a DIFFERENT return type (`Integer`, not
    // the catalog's `Text`) shadows the global builtin. `resolve_bare`
    // resolves this to `RouteTarget::Routine` via Step 1 (own object) —
    // Step 4's builtin fallback (and therefore the passive builtin-return
    // catalog) is structurally UNREACHABLE for this name here. If the
    // catalog were ever consulted by NAME STRING ALONE (a bug this fixture
    // catches), the outer call would wrongly pick `Q(S: Text)`; the CORRECT
    // behavior picks `Q(N: Integer)` (this shadowing `Format`'s real
    // declared return type).
    procedure RunShadowedFormat()
    var
        T: Codeunit "CRN Target";
    begin
        T.Q(Format());
    end;

    local procedure Format(): Integer
    begin
        exit(42);
    end;

    // (c) SHADOWED-NAME fixture (mandatory), the OTHER named pair: a SOURCE
    // `CopyStr` returning `Integer` shadows the catalog's `Text` entry.
    procedure RunShadowedCopyStr()
    var
        T: Codeunit "CRN Target";
    begin
        T.R(CopyStr());
    end;

    local procedure CopyStr(): Integer
    begin
        exit(7);
    end;
}
