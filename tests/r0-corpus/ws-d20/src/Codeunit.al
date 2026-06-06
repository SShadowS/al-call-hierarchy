codeunit 50600 "D20 Demo"
{
    // FLAGGED: statement after Exit; in same code_block.
    procedure ExitThenStatement()
    begin
        Exit;
        Message('never runs');
    end;

    // FLAGGED: Error then statement at same level.
    procedure ErrorThenStatement()
    begin
        Error('oops');
        Message('never runs');
    end;

    // NOT FLAGGED: Exit inside an if-branch is conditional; the sibling after the
    // if-statement IS reachable when the condition is false.
    procedure ConditionalExit(x: Integer)
    begin
        if x > 0 then
            Exit;
        Message('reachable when x <= 0');
    end;

    // NOT FLAGGED: Exit is the last statement of the block.
    procedure ExitAtEnd()
    begin
        Message('runs');
        Exit;
    end;
}
