codeunit 50316 "Variable Receiver Caller"
{
    // A `var` PARAMETER receiver — CDO's exact shape (Codeunit 6175274
    // "CDO Continia Online PDF Mgt".MergePdf's `var DOFile: Record "CDO
    // File"` calling `DOFile.IsPdf()`). Legacy's `variable_bindings` is
    // populated ONLY from a routine's `var`-section locals
    // (`push_variables_ir(&mut result, &r.locals, ...)`,
    // `src/parser.rs:293`) — a signature PARAMETER (`r.params`) never
    // flows through it at all, so `lookup_variable_type` can never type
    // this receiver.
    procedure MergePdf(var DOFile: Record "Variable Receiver Table")
    begin
        DOFile.IsPdf();
    end;

    // A LOCAL `var`-section variable receiver — legacy's
    // `push_variables_ir` DOES capture this shape, so `lookup_variable_type`
    // should resolve it correctly (a genuine MATCH expected here, NOT a
    // divergence — see the task report for why this fixture arm was built
    // to verify, not assume, that distinction).
    procedure UseLocalVar()
    var
        Y: Record "Variable Receiver Table";
    begin
        Y.IsPasswordProtected();
    end;
}
