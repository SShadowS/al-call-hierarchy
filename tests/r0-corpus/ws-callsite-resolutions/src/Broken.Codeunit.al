codeunit 50101 "CSR Broken"
{
    procedure Incomplete()
    begin
        CallSomething(); @@@  // stray @ token — intentional parse error inside body
    end;
}
