codeunit 50000 Publisher
{
    procedure FirePost(var W: Record Widget)
    begin
        OnAfterPost(W);
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterPost(var W: Record Widget)
    begin
    end;
}
