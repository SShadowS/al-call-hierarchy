codeunit 50115 D14PublicDead
{
    // Entry point reaches LiveHelper only.
    trigger OnRun()
    begin
        LiveHelper();
    end;

    local procedure LiveHelper() begin end;

    // Demonstrably scope-limited and unreached → must be flagged.
    local procedure DeadLocal() begin end;

    // Default access (public): might be called from another app — must NOT be flagged.
    procedure DeadPublic() begin end;

    // Public wrapper that only delegates to a local helper. The wrapper is a
    // reachable root (callable from outside), so the helper is transitively reached.
    procedure WrapperPublic()
    begin
        WrappedLocal();
    end;

    // Local helper called only through WrapperPublic. Must NOT be flagged — its
    // caller is a reachable root.
    local procedure WrappedLocal() begin end;

    // Internal: visible to internalsVisibleTo apps — must NOT be flagged.
    internal procedure DeadInternal() begin end;

    // Protected: visible to overriding codeunits — must NOT be flagged.
    protected procedure DeadProtected() begin end;
}
