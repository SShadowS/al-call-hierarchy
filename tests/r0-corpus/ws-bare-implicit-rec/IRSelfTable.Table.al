// beyond-1B.3b Task 3 fixture (i), SHADOW-GUARD (NOT a Step-3 proof): `Run`
// calls bare `Recalc()` from within Recalc's OWN table (`"IR Self Table"`).
// This resolves via Step 1 (own object) — Step 3 (implicit-Rec) is never
// reached, since for a Table object kind the "implicit table" IS the object
// itself, so Step 1 and Step 3 would produce the SAME target here; this
// fixture documents that Step 1 short-circuits FIRST, exercising the
// pre-existing own-object precedence rather than the new Step 3 guard.
table 50986 "IR Self Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure Recalc(): Text
    begin
        exit('recalc');
    end;

    procedure Run()
    begin
        Recalc();
    end;
}
