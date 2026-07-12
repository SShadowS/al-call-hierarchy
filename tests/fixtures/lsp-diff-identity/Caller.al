// Variable names match each object's real name exactly (see
// tests/fixtures/lsp-diff-deps/Caller.al's own comment for why: legacy's
// `outgoing_calls` resolves a qualified call by the call site's OWN raw
// receiver TEXT, never re-resolving a differently-named local variable to
// its declared type).
codeunit 50305 "Identity Caller"
{
    procedure CallCodeunitVersion(): Text
    var
        "Shared Name": Codeunit "Shared Name";
    begin
        exit("Shared Name".GetRecipients());
    end;

    procedure CallPageVersion(): Text
    var
        "Shared Name": Page "Shared Name";
    begin
        exit("Shared Name".GetRecipients());
    end;
}
