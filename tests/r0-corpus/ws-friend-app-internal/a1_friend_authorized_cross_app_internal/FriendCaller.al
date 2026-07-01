// App: PrimaryAppFriend, depends on DepAppFriend. FriendTarget.Secret() is
// `internal` but DepAppFriend's manifest lists PrimaryAppFriend as a friend
// -> resolves to Source (Task 1.5). Pre-fix (Task 1 alone, without friend
// modeling) this was an over-decline to Unknown/InternalNotVisible.
codeunit 53971 "FriendCaller"
{
    procedure Trigger()
    begin
    end;
}
