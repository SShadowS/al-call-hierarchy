report 50310 "Nested Report Trigger"
{
    UsageCategory = ReportsAndAnalysis;

    dataset
    {
        dataitem(NestedTriggerTable; "Nested Trigger Table")
        {
            trigger OnAfterGetRecord()
            begin
                HandleAfterGetRecord();
            end;
        }
    }

    procedure HandleAfterGetRecord()
    begin
    end;
}
