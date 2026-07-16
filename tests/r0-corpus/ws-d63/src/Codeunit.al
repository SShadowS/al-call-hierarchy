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

    // NOT FLAGGED: a multi-line HTML template joined with `+` where every operand
    // is a static literal — dynamic values enter via StrSubstNo %n placeholders,
    // not via concatenation (the DO false-positive shape: the StrSubstNo call
    // site's template argument is pure-literal concat; %1/%2 are placeholders).
    procedure BuildTemplate(Name: Text): Text
    begin
        exit(StrSubstNo(
            '<div><p><br></p></div>' +
            '<div><b>%1</b> &lt;%2&gt;<br></div>',
            'Name:', Name));
    end;

    local procedure Render(Html: Text): Text
    begin
        exit(Html);
    end;
}
