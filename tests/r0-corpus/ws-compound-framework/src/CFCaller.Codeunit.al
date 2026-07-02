// beyond-1B.3b Task 4 fixtures — `<Framework>.<Prop|Method()>` compound-receiver
// resolution (see `infer_receiver_type_for_expr`/`infer_compound_member_receiver`,
// `src/program/resolve/receiver.rs`) + `this.<rest>` self-scoped stripping
// (`infer_this_member`). One PROCEDURE per scenario so `edges_for_object_routine`
// can isolate each call obligation cleanly, mirroring
// `tests/r0-corpus/ws-compound-call-result/`'s layout (Task 3).
codeunit 51101 "CF Caller"
{
    // `this.DialogWindow.Open()` (fixture c) addresses this OBJECT GLOBAL —
    // `this.` deliberately never sees locals/params (see `infer_this_member`'s
    // doc), only globals like this one.
    var
        DialogWindow: Dialog;

    // ---- POSITIVES ------------------------------------------------------

    // (a) `Response: HttpResponseMessage` → `Content()` (real AL zero-arg
    // method, table-verified against methods-auto/httpresponsemessage) →
    // `HttpContent` — `ReadAs` (a real HttpContent member) must resolve via
    // the ordinary Framework(HttpContent) catalog route.
    procedure TestHttpResponseContent()
    var
        Response: HttpResponseMessage;
        Body: Text;
    begin
        Response.Content().ReadAs(Body);
    end;

    // (b) `JToken: JsonToken` → `AsObject()` (table-verified) → `JsonObject`
    // — `Get` (a real JsonObject member) must resolve via the Framework
    // (JsonObject) catalog route.
    procedure TestJsonTokenAsObject()
    var
        JToken: JsonToken;
        Found: JsonToken;
    begin
        JToken.AsObject().Get('key', Found);
    end;

    // (c) `this`-strip: `DialogWindow` resolved against the OBJECT-GLOBAL-only
    // self scope (never `routine.params`/`routine.locals`) → `Framework(Dialog)`
    // — `Open` (a real Dialog member) must resolve via the catalog route.
    procedure TestThisStripDialogWindow()
    begin
        this.DialogWindow.Open();
    end;

    // ---- NEGATIVES (all must stay honest Unknown) ------------------------

    // (d) Base not a known framework type: `Foo` is not declared anywhere
    // reachable from this object — the recursive base-typing declines, so the
    // whole chain declines.
    procedure TestBaseNotFramework()
    var
        Body: Text;
    begin
        Foo.Content().ReadAs(Body);
    end;

    // (e) Table-miss: `Response` types `Framework(HttpResponseMessage)` but
    // `"Bar"` is not a table entry for that kind — fail-closed (never
    // fabricate a returned kind for an unlisted member).
    procedure TestTableMiss()
    var
        Response: HttpResponseMessage;
        Body: Text;
    begin
        Response.Bar().ReadAs(Body);
    end;

    // (f) Wrong FORM: a table METHOD-entry (`HttpResponseMessage.Content()`,
    // `is_method: true`) invoked as a PROPERTY (no parens) — never matches.
    procedure TestWrongFormPropertyInsteadOfMethod()
    var
        Response: HttpResponseMessage;
        Body: Text;
    begin
        Response.Content.ReadAs(Body);
    end;

    // (g) Wrong ARITY: `Response.Content(X)` (1 arg) never matches the
    // table's arity-0 entry.
    procedure TestWrongArity()
    var
        Response: HttpResponseMessage;
        X: HttpContent;
        Body: Text;
    begin
        Response.Content(X).ReadAs(Body);
    end;

    // (h) A base whose recursion mis-types: `Response.Bar()` is itself a
    // table-miss (declines to Unknown), so the OUTER `.Content()` hop's base
    // is Unknown (not Framework) — the whole chain declines, proving a
    // mis-typed intermediate hop propagates rather than resetting to a guess.
    procedure TestRecursionMistype()
    var
        Response: HttpResponseMessage;
        Body: Text;
    begin
        Response.Bar().Content().ReadAs(Body);
    end;

    // (i) A same-named member on a NON-framework type must NOT hit the table:
    // `Cust: Record "CF Customer"` types `Record{..}`, not `Framework` — the
    // table lookup never engages (short-circuited by the Framework-only
    // guard), even though `"content"` happens to be a valid HttpResponseMessage
    // table member.
    procedure TestSameNamedMemberOnNonFrameworkBase()
    var
        Cust: Record "CF Customer";
        Body: Text;
    begin
        Cust.Content().ReadAs(Body);
    end;

    // (j) DEFERRED-shape guard: record-field member-of-member —
    // `Rec.BlobField.CreateOutStream()` stays Unknown. `Rec` types
    // `Record{..}`, not `Framework` — field-type indexing (`BlobField`'s
    // declared field TYPE) is a genuinely different, deferred mechanism
    // (node-model-heavy, out of this task's scope).
    procedure TestDeferredRecordField()
    var
        Rec: Record "CF Customer";
    begin
        Rec.BlobField.CreateOutStream();
    end;
}
