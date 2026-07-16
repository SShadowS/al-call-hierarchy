codeunit 50940 "D63 Demo"
{
    // FLAGGED: HTML literal concatenated with data — no HtmlEncode in AL.
    procedure BuildUnsafe(UserName: Text): Text
    begin
        exit(Render('<b>' + UserName + '</b>'));
    end;

    // NOT FLAGGED: static HTML, no concatenation.
    procedure BuildStatic(): Text
    begin
        exit(Render('<b>static</b>'));
    end;

    // NOT FLAGGED: concatenation without HTML literals.
    procedure BuildPlain(UserName: Text): Text
    begin
        exit(Render('Hello ' + UserName));
    end;

    local procedure Render(Html: Text): Text
    begin
        exit(Html);
    end;
}
