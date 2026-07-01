// Case (j) PEER-extension `Protected` BLEED — the biggest latent
// false-`Source` this task closes. `BarExtB` is a SIBLING extension of the
// SAME base (`Bar`) as `BarExtA` (see BarExtA.TableExt.al), NOT an
// extension of `BarExtA` itself — extensions can only ever `extends` the
// BASE object, never a peer extension.
//
// AL semantics: DOES NOT COMPILE (access error — `P` is `protected` on
// `BarExtA`, and `BarExtB` extends `Bar`, not `BarExtA`; `Record Bar`'s
// visible surface for `BarExtB` does not include `BarExtA`'s protected
// members).
//
// Fresh-engine expected route (POST-FIX): Evidence::Unknown. PRE-FIX (the
// bug this task closes): Evidence::Source, wrongly targeting BarExtA.P —
// see `resolve_member_record_peer_extension_protected_bleed_excluded` and
// COMPILER_PROOF.md row (j).
tableextension 52700 "BarExtB" extends Bar
{
    procedure Wrapper()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
