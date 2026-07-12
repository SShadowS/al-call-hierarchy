// VariableReceiverResolved probe (CDO layer 4): a codeunit calling ITSELF
// through a `var` PARAMETER of its own type — mirrors CDO `Codeunit 6175324
// "CDO XML Node"`'s `AddNode(var NewXmlNode: Codeunit "CDO XML Node" ...)`
// body calling `NewXmlNode.SetXmlNode(...)`, where caller and callee are
// BOTH declared in the same codeunit. The old layer-3 predicate required
// `caller object != callee object`, so this same-object shape never
// reached it; legacy still can't resolve it, since `variable_bindings`
// never binds a signature parameter regardless of object identity.
codeunit 50317 "Same Object Var Receiver"
{
    procedure AddNode(var NewNode: Codeunit "Same Object Var Receiver")
    begin
        NewNode.SetNode();
    end;

    procedure SetNode()
    begin
    end;
}
