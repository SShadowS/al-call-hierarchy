codeunit 50101 "Probe Sha"
{
    procedure InsertWithSha1(L: Interface "Probe ILog"; S: Text; Acc: Code[20]): Integer
    begin
        exit(0);
    end;

    procedure InsertWithSha1(L: Interface "Probe ILog"; S: InStream; Acc: Code[20]): Integer
    var
        Rec: Record "Probe File";
    begin
        Rec.Init();
        Rec.Insert(true);
        Commit();
        exit(1);
    end;

    procedure M(Tok: Integer; S: Text; Acc: Code[20]): Integer
    begin
        exit(0);
    end;

    procedure M(Tok: Integer; S: InStream; Acc: Code[20]): Integer
    begin
        Commit();
        exit(1);
    end;
}
