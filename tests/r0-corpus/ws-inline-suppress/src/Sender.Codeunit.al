codeunit 50001 "IS Sender"
{
    // Procedure 1: D47 finding suppressed by an inline directive on the line above the IO call.
    // The directive names the correct detector id with a reason.
    procedure SuppressedIo()
    var
        Rec: Record "IS Rec";
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        // al-sem-ignore d47-io-unsafe-txn: reviewed, idempotent endpoint
        Client.Get('https://example.test/ping', Resp);
    end;

    // Procedure 2: D47 finding NOT suppressed — directive names a DIFFERENT detector id.
    procedure WrongDirectiveIo()
    var
        Rec2: Record "IS Rec";
        Client2: HttpClient;
        Resp2: HttpResponseMessage;
    begin
        Rec2.Get(20000);
        Rec2.Name := 'other';
        Rec2.Modify();
        // al-sem-ignore d1-db-op-in-loop: wrong detector, should not suppress d47
        Client2.Get('https://example.test/other', Resp2);
    end;

    // Procedure 3: D3 finding (Get without SetLoadFields in a loop) — NOT suppressed.
    // This produces an unrelated finding that inline suppression must NOT touch.
    procedure UnsuppressedD3()
    var
        Rec3: Record "IS Rec";
    begin
        if Rec3.FindSet() then
            repeat
                Rec3.Get(Rec3."No.");
            until Rec3.Next() = 0;
    end;
}
