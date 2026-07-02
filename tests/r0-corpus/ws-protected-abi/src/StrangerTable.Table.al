// Task 1 fixture (h) support: a WORKSPACE table that deliberately collides
// with the dep ABI's "Dep Page" on BOTH numeric id (60000) AND name, but is a
// DIFFERENT KIND (Table, not Page) and an entirely different, unrelated
// declaration (zero procedures of its own). `ObjectNodeId` keys on
// `(app, kind, key)`, so this is a genuinely distinct identity from the ABI
// Page despite the id/name collision — proving `object_extends`/
// `resolve_object_ref` never conflate them by id/name alone.
table 60000 "Dep Page"
{
}
