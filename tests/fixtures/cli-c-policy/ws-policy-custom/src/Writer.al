// ModifyOrder: modifies a local table — name resolves to "Custom Order"
//   → capability.resource.table.name glob "* Order" = true (match)
//   Used by: cust-glob-table rule (glob operator, table name glob match)
//
// ReadExternal: reads an external table not defined in the workspace
//   → capability.resource.table.name = unknown (resource-id-unresolved)
//   Used by: cust-unknown-table rule with onUnknown:fail-closed → unknown-finding
//
// DispatchDynamic: performs a dynamic Codeunit.Run via a table field
//   → coverage.inheritedStatus = "partial" (unresolved target)
//   Used by: cust-coverage-complete rule with requireCoverage:complete, onUnknown:fail-open
//   → skippedCoverage but no finding (fail-open)
//
// DoNothing: no capability facts → no match, no finding (false/pass)
codeunit 50202 "Custom Writer"
{
    procedure ModifyOrder()
    var
        Ord: Record "Custom Order";
    begin
        Ord.Get('ORD001');
        Ord.Description := 'Updated';
        Ord.Modify(true);
    end;

    procedure ReadExternal()
    var
        Ledger: Record "G/L Entry";
    begin
        // "G/L Entry" is not defined in this workspace — resourceId will be undefined
        // → capability.resource.table.name = unknown("resource-id-unresolved")
        Ledger.FindSet();
    end;

    procedure DispatchDynamic()
    var
        Ord: Record "Custom Order";
    begin
        Ord.Get('ORD001');
        // Dynamic dispatch: target not statically resolvable → partial coverage
        Codeunit.Run(Ord."No.");
    end;

    procedure DoNothing()
    var
        i: Integer;
    begin
        i := 1 + 1;
    end;
}
