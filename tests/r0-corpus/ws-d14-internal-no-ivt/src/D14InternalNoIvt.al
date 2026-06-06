codeunit 50116 D14InternalNoIvt
{
    // Entry point — reaches LiveHelper transitively via the internal wrapper.
    trigger OnRun()
    begin
        LiveInternal();
    end;

    // Internal: reached from OnRun → must NOT be flagged even with no IVT.
    internal procedure LiveInternal()
    begin
        LiveHelper();
    end;

    local procedure LiveHelper() begin end;

    // Internal with no IVT and no in-app caller → demonstrably dead, MUST be flagged.
    internal procedure DeadInternal() begin end;

    // Local control: also dead, MUST be flagged (existing behavior).
    local procedure DeadLocal() begin end;

    // Public stays a root regardless of IVT — must NOT be flagged.
    procedure DeadPublic() begin end;

    // Protected stays a root regardless of IVT — must NOT be flagged.
    protected procedure DeadProtected() begin end;
}
