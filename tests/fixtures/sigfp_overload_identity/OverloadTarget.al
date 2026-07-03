// Fixture for the sigfp-and-ambiguous-reclassification plan, Task 2 —
// real source `sig_fp` identity, exercised end-to-end through all 4 live
// reconstruction sites (node_extract::extract_nodes, resolve::body_map::
// BodyMap::build, resolve::full::resolve_full_program_from_parts,
// resolve::stub::resolve_program) via ONE shared constructor
// (sig_fp::source_routine_node_id).
//
// Two genuine same-name/same-arity overloads, differing only by parameter
// TYPE, each calling a DIFFERENT, uniquely-named helper — proves per-overload
// caller attribution (the outgoing HelperInt()/HelperText() call must
// attribute to its OWN overload's id, never merged/confused with its
// sibling's).
codeunit 50990 "SigFp Overload Target"
{
    procedure Resolve(Value: Integer)
    begin
        HelperInt();
    end;

    procedure Resolve(Value: Text)
    begin
        HelperText();
    end;

    procedure HelperInt()
    begin
    end;

    procedure HelperText()
    begin
    end;
}
