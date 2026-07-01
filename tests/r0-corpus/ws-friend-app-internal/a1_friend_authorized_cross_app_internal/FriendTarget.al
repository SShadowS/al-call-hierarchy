// App: DepAppFriend. This app's manifest declares "PrimaryAppFriend" (the
// caller's app) a friend via <InternalsVisibleTo><Module .../></...>. AL:
// an `internal` member is visible within its declaring app AND to any app
// the declaring app's manifest lists as a friend — this call is AL-LEGAL.
codeunit 53970 "FriendTarget"
{
    internal procedure Secret()
    begin
    end;
}
