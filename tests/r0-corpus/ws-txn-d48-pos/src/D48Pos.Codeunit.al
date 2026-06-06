codeunit 50000 "D48 Sender"
{
    /// <summary>
    /// Sends one HTTP POST. Top-level call — no loop here.
    /// When invoked from within a loop this becomes the transitive terminal.
    /// Also invoked from a non-loop site in D48 Caller (caller-context sensitivity).
    /// </summary>
    procedure SendOne()
    var
        Client: HttpClient;
        Req: HttpRequestMessage;
        Resp: HttpResponseMessage;
    begin
        Req.Method := 'POST';
        Client.Send(Req, Resp);
    end;
}

codeunit 50001 "D48 Loop Caller"
{
    /// <summary>
    /// Calls SendOne() in a repeat..until loop — transitive HTTP-in-loop.
    /// D48 must flag this routine.
    /// </summary>
    procedure ProcessAll()
    var
        i: Integer;
    begin
        i := 0;
        repeat
            i += 1;
            SendOne();
        until i >= 10;
    end;

    local procedure SendOne()
    var
        Sender: Codeunit "D48 Sender";
    begin
        Sender.SendOne();
    end;
}

codeunit 50002 "D48 Non Loop Caller"
{
    /// <summary>
    /// Calls SendOne() exactly once, outside any loop.
    /// D48 must NOT flag this routine.
    /// </summary>
    procedure DoSendOnce()
    var
        Sender: Codeunit "D48 Sender";
    begin
        Sender.SendOne();
    end;
}

codeunit 50003 "D48 File Loop"
{
    /// <summary>
    /// Writes a file inside a repeat..until loop — direct FILE-in-loop.
    /// File.WriteAllText produces a resourceKind="file" direct capability fact.
    /// D48 must flag this routine at MEDIUM severity (FILE → medium).
    /// </summary>
    procedure ExportAll()
    var
        F: File;
        i: Integer;
    begin
        i := 0;
        repeat
            i += 1;
            F.WriteAllText('line', TextEncoding::UTF8);
        until i >= 10;
    end;
}
