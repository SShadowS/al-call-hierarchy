codeunit 50000 "D49 Sender"
{
    /// <summary>
    /// NEGATIVE: Message() with no preceding physical write — no pending write.
    /// Must produce ZERO d49 findings.
    /// </summary>
    procedure MessageOnly()
    begin
        Message('Hello');
    end;
}
