codeunit 50100 "Test Caller"
{
    procedure RunTest()
    begin
        // Qualified call with no local var, no local object, no external dep -
        // hits the ObjectNotFound fall-through in graph.rs::resolve_call.
        "Missing External Codeunit".PostInvoice('CUST001', 100);

        // Unqualified call to a procedure that does not exist in this object -
        // hits the UnresolvedUnqualified case in graph.rs::resolve_call.
        SomeMissingHelper(42);
    end;
}
