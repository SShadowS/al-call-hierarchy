// T0.3 fixture (t0-task-3-brief.md): exercises the builtin-dispatch
// justification audit's two flagged populations plus its fail-closed
// indeterminate path. See `member_catalog::ENTRY_DISPATCH_BUILTIN_IDS`'s doc
// for the two classifier gaps this audit makes visible.
codeunit 50953 "Audit Caller"
{
    /// FLAGGED — the brief's cited shape: a keyword receiver (`Page` used as
    /// a pseudo-namespace) with a `DatabaseReference` first argument. The
    /// classifier gate (extract.rs Check 2) only recognizes method "run",
    /// not "runmodal", so this resolves as an ordinary Catalog-evidence
    /// Builtin route (`PageInstance::runmodal`) instead of an entry-trigger
    /// Run edge into "Audit Target Page".
    procedure FlaggedPageKeywordRunModal()
    begin
        Page.RunModal(Page::"Audit Target Page");
    end;

    /// FLAGGED — same shape, Report.RunModal.
    procedure FlaggedReportKeywordRunModal()
    begin
        Report.RunModal(Report::"Audit Target Report");
    end;

    /// FLAGGED — a declared Page-typed variable receiver. RunModal isn't a
    /// declared procedure of "Audit Target Page", so resolution falls
    /// through to the SAME instance-builtin catalog. The target is
    /// statically known from the variable's DECLARED TYPE — no argument
    /// inspection needed.
    procedure FlaggedDeclaredPageVarRunModal()
    var
        MyPage: Page "Audit Target Page";
    begin
        MyPage.RunModal();
    end;

    /// INDETERMINATE — the target is a runtime variable, not a statically
    /// named `Page::"X"` reference: the audit must NOT guess and must NOT
    /// count this as flagged.
    procedure IndeterminatePageKeywordDynamicTarget()
    var
        DynamicPageId: Integer;
    begin
        DynamicPageId := 50951;
        Page.RunModal(DynamicPageId);
    end;
}
