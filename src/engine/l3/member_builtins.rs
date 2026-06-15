//! Intrinsic built-in catalog (spec §4) — the data-driven
//! `(receiver-kind, method_lc) -> Disposition` table for AL's COMPILER-INTRINSIC
//! member methods on Record / RecordRef / FieldRef / KeyRef + the framework data
//! types (Json*, Http*, In/OutStream, TextBuilder, Dialog, List, Dictionary,
//! Xml*). These methods are baked into the AL compiler (NOT shipped in any `.app`
//! `SymbolReference.json`), so this catalog is a separate, hand-built knowledge
//! asset, looked up via `phf` perfect-hash for zero-cost recognition.
//!
//! Phase-2 contract: a catalog hit on the MEMBER path classifies the edge as
//! `builtin` (a platform terminal, NOT a resolution hole). The `Disposition`
//! distinguishes plain builtins from `FlowsType` builtins (RecordRef
//! `GetTable`/`Open`/`SetTable`) which a later phase (§5 TableID const-prop) will
//! turn dynamic->static; in Phase 2 BOTH dispositions emit `builtin`.

use phf::{phf_map, phf_set};

/// The receiver kinds that have an intrinsic built-in catalog. (Object types —
/// Codeunit/Page/Report/Query/XmlPort/Interface — and Enum/Primitive are handled
/// on other paths and are intentionally NOT here.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverBuiltinKind {
    Record,
    RecordRef,
    FieldRef,
    KeyRef,
    JsonObject,
    JsonToken,
    JsonArray,
    JsonValue,
    HttpClient,
    HttpRequestMessage,
    HttpResponseMessage,
    HttpHeaders,
    HttpContent,
    InStream,
    OutStream,
    TextBuilder,
    Dialog,
    List,
    Dictionary,
    Xml,
    /// The current page instance — `CurrPage.M()` calls inside a page trigger.
    /// Methods come from the AL compiler's `Page` instance member catalog.
    PageInstance,
    /// The current report instance — `CurrReport.M()` calls inside a report
    /// trigger. Methods come from the AL compiler's `ReportInstance` catalog.
    ReportInstance,
    /// AL platform singleton `IsolatedStorage` — static key/value store.
    /// Source: member_builtins.json "IsolatedStorage" (5 methods).
    IsolatedStorage,
    /// AL platform singleton `Session` — session utilities, telemetry, bindings.
    /// Source: member_builtins.json "Session" (19 methods).
    Session,
    /// AL platform singleton `NavApp` — extension/module info and resource APIs.
    /// Source: member_builtins.json "NavApp" (16 methods).
    NavApp,
    /// AL platform singleton `TaskScheduler` — background task scheduling.
    /// Source: member_builtins.json "TaskScheduler" (5 methods).
    TaskScheduler,
    /// AL platform singleton `Database` — low-level database utilities.
    /// Source: member_builtins.json "Database" (29 methods).
    Database,
    /// A `Blob`-typed table FIELD used as a member receiver (e.g.
    /// `"File Blob".CreateInStream(...)`). Blob fields expose the stream-creation
    /// intrinsics; they are not a declared variable type, so this kind is produced
    /// from a field-type lookup, not `classify_receiver`.
    Blob,
    /// A `Media` / `MediaSet`-typed table FIELD used as a member receiver — the
    /// media import/export/query intrinsics.
    Media,
    /// A page control-add-in (usercontrol) receiver — `CurrPage.<addin>.<method>()`.
    /// Add-in methods are platform/JS calls with no in-AL target; every method
    /// classifies `builtin`.
    ControlAddIn,
}

/// How a catalog-recognized member method dispatches. Phase 2 emits `builtin` for
/// BOTH; `FlowsType` is data-only marking for the §5 dynamic->static work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Platform method with no AL target and no type flow.
    Builtin,
    /// Platform method that flows a record/table type into its receiver
    /// (RecordRef `Open`/`GetTable`/`SetTable`). Emitted as `builtin` in Phase 2.
    FlowsType,
}

/// Classify a normalized `declared_type` string into a builtin-catalog receiver
/// kind, or `None` if it is an object type / primitive / unrecognized type. The
/// input is already whitespace-collapsed with quotes preserved (the L2
/// `canonicalize_type_text` shape). Pure, never panics.
pub fn classify_receiver(declared_type: &str) -> Option<ReceiverBuiltinKind> {
    let dt = declared_type.trim();
    if dt.is_empty() {
        return None;
    }
    let first = match dt.find(' ') {
        Some(i) => &dt[..i],
        None => dt,
    };
    let lc = first.to_lowercase();
    Some(match lc.as_str() {
        "record" => ReceiverBuiltinKind::Record,
        "recordref" => ReceiverBuiltinKind::RecordRef,
        "fieldref" => ReceiverBuiltinKind::FieldRef,
        "keyref" => ReceiverBuiltinKind::KeyRef,
        "jsonobject" => ReceiverBuiltinKind::JsonObject,
        "jsontoken" => ReceiverBuiltinKind::JsonToken,
        "jsonarray" => ReceiverBuiltinKind::JsonArray,
        "jsonvalue" => ReceiverBuiltinKind::JsonValue,
        "httpclient" => ReceiverBuiltinKind::HttpClient,
        "httprequestmessage" => ReceiverBuiltinKind::HttpRequestMessage,
        "httpresponsemessage" => ReceiverBuiltinKind::HttpResponseMessage,
        "httpheaders" => ReceiverBuiltinKind::HttpHeaders,
        "httpcontent" => ReceiverBuiltinKind::HttpContent,
        "instream" => ReceiverBuiltinKind::InStream,
        "outstream" => ReceiverBuiltinKind::OutStream,
        "textbuilder" => ReceiverBuiltinKind::TextBuilder,
        "blob" => ReceiverBuiltinKind::Blob,
        "media" | "mediaset" => ReceiverBuiltinKind::Media,
        "dialog" => ReceiverBuiltinKind::Dialog,
        "list" => ReceiverBuiltinKind::List,
        "dictionary" => ReceiverBuiltinKind::Dictionary,
        s if s.starts_with("xml") => ReceiverBuiltinKind::Xml,
        _ => return None,
    })
}

/// The disposition of `(kind, method_lc)` if it is a recognized intrinsic, else
/// `None`. `method_lc` MUST be lowercase. Pure, never panics.
pub fn member_builtin_disposition(
    kind: ReceiverBuiltinKind,
    method_lc: &str,
) -> Option<Disposition> {
    use ReceiverBuiltinKind::*;
    match kind {
        RecordRef => RECORDREF.get(method_lc).copied(),
        Record => set_hit(&RECORD, method_lc),
        FieldRef => set_hit(&FIELDREF, method_lc),
        KeyRef => set_hit(&KEYREF, method_lc),
        JsonObject => set_hit(&JSONOBJECT, method_lc),
        JsonToken => set_hit(&JSONTOKEN, method_lc),
        JsonArray => set_hit(&JSONARRAY, method_lc),
        JsonValue => set_hit(&JSONVALUE, method_lc),
        HttpClient => set_hit(&HTTPCLIENT, method_lc),
        HttpRequestMessage => set_hit(&HTTPREQUEST, method_lc),
        HttpResponseMessage => set_hit(&HTTPRESPONSE, method_lc),
        HttpHeaders => set_hit(&HTTPHEADERS, method_lc),
        HttpContent => set_hit(&HTTPCONTENT, method_lc),
        InStream => set_hit(&INSTREAM, method_lc),
        OutStream => set_hit(&OUTSTREAM, method_lc),
        TextBuilder => set_hit(&TEXTBUILDER, method_lc),
        Dialog => set_hit(&DIALOG, method_lc),
        List => set_hit(&LIST, method_lc),
        Dictionary => set_hit(&DICTIONARY, method_lc),
        Xml => set_hit(&XML, method_lc),
        PageInstance => set_hit(&PAGE_INSTANCE, method_lc),
        ReportInstance => set_hit(&REPORT_INSTANCE, method_lc),
        IsolatedStorage => set_hit(&ISOLATED_STORAGE, method_lc),
        Session => set_hit(&SESSION, method_lc),
        NavApp => set_hit(&NAVAPP, method_lc),
        TaskScheduler => set_hit(&TASKSCHEDULER, method_lc),
        Database => set_hit(&DATABASE, method_lc),
        Blob => set_hit(&BLOB, method_lc),
        Media => set_hit(&MEDIA, method_lc),
        // ANY method on a control-add-in is a builtin: we cannot enumerate an
        // add-in's JS method surface, and these are genuine platform calls, never
        // real-`unknown`.
        ControlAddIn => Some(Disposition::Builtin),
    }
}

#[inline]
fn set_hit(set: &phf::Set<&'static str>, method_lc: &str) -> Option<Disposition> {
    if set.contains(method_lc) {
        Some(Disposition::Builtin)
    } else {
        None
    }
}

// --- Record (the largest CDO bucket). AL forbids overriding built-ins, so these
//     never collide with user table procedures. ---
static RECORD: phf::Set<&'static str> = phf_set! {
    "get", "getbysystemid", "find", "findfirst", "findlast", "findset", "next",
    "insert", "modify", "delete", "deleteall", "modifyall", "init", "rename",
    "validate", "calcfields", "calcsums", "setautocalcfields",
    "setrange", "setfilter", "getfilter", "getfilters", "setview", "getview",
    "getrangemin", "getrangemax", "getrangefilter", "copyfilter", "copyfilters",
    "setcurrentkey", "currentkey", "currentkeyindex", "ascending", "reset",
    "copy", "count", "countapprox", "isempty", "hasfilter", "filtergroup",
    "markedonly", "mark", "marked", "clearmarks", "setrecfilter", "setpermissionfilter",
    "fieldno", "fieldname", "fieldcaption", "fieldactive", "fielderror",
    "testfield", "fieldexist", "tablecaption", "recordid", "getposition",
    "setposition", "transferfields", "addlink", "deletelink", "deletelinks",
    "copylinks", "haslinks", "locktable", "consistent", "changecompany",
    "readpermission", "writepermission", "setloadfields", "addloadfields",
    "areanyfieldsmodified", "getascending",
    // Added from compiler JSON:
    "arefieldsloaded", "currentcompany", "fullyqualifiedname", "istemporary",
    "loadfields", "readconsistency", "readisolation", "recordlevellocking",
    "relation", "securityfiltering", "setascending", "setbaseloadfields",
    "tablename", "truncate",
};

// --- RecordRef (Map: Open/GetTable/SetTable flow a table type -> FlowsType). ---
static RECORDREF: phf::Map<&'static str, Disposition> = phf_map! {
    "open" => Disposition::FlowsType,
    "openshared" => Disposition::FlowsType,
    "gettable" => Disposition::FlowsType,
    "settable" => Disposition::FlowsType,
    "close" => Disposition::Builtin,
    "number" => Disposition::Builtin,
    "name" => Disposition::Builtin,
    "caption" => Disposition::Builtin,
    "fieldcount" => Disposition::Builtin,
    "field" => Disposition::Builtin,
    "fieldindex" => Disposition::Builtin,
    "fieldexist" => Disposition::Builtin,
    "keycount" => Disposition::Builtin,
    "keyindex" => Disposition::Builtin,
    "currentkeyindex" => Disposition::Builtin,
    "init" => Disposition::Builtin,
    "insert" => Disposition::Builtin,
    "modify" => Disposition::Builtin,
    "delete" => Disposition::Builtin,
    "deleteall" => Disposition::Builtin,
    "modifyall" => Disposition::Builtin,
    "find" => Disposition::Builtin,
    "findfirst" => Disposition::Builtin,
    "findlast" => Disposition::Builtin,
    "findset" => Disposition::Builtin,
    "next" => Disposition::Builtin,
    "setrange" => Disposition::Builtin,
    "setfilter" => Disposition::Builtin,
    "getview" => Disposition::Builtin,
    "setview" => Disposition::Builtin,
    "getfilters" => Disposition::Builtin,
    "reset" => Disposition::Builtin,
    "copy" => Disposition::Builtin,
    "count" => Disposition::Builtin,
    "countapprox" => Disposition::Builtin,
    "isempty" => Disposition::Builtin,
    "ascending" => Disposition::Builtin,
    "setpermissionfilter" => Disposition::Builtin,
    "addloadfields" => Disposition::Builtin,
    "setloadfields" => Disposition::Builtin,
    "hasfilter" => Disposition::Builtin,
    "markedonly" => Disposition::Builtin,
    "mark" => Disposition::Builtin,
    "recordid" => Disposition::Builtin,
    "getposition" => Disposition::Builtin,
    "setposition" => Disposition::Builtin,
    "filtergroup" => Disposition::Builtin,
    "changecompany" => Disposition::Builtin,
    "calcfields" => Disposition::Builtin,
    "calcsums" => Disposition::Builtin,
    // Added from compiler JSON:
    "addlink" => Disposition::Builtin,
    "arefieldsloaded" => Disposition::Builtin,
    "clearmarks" => Disposition::Builtin,
    "copylinks" => Disposition::Builtin,
    "currentcompany" => Disposition::Builtin,
    "currentkey" => Disposition::Builtin,
    "duplicate" => Disposition::Builtin,
    "fullyqualifiedname" => Disposition::Builtin,
    "getbysystemid" => Disposition::Builtin,
    "istemporary" => Disposition::Builtin,
    "isdirty" => Disposition::Builtin,
    "loadfields" => Disposition::Builtin,
    "locktable" => Disposition::Builtin,
    "readconsistency" => Disposition::Builtin,
    "readisolation" => Disposition::Builtin,
    "readpermission" => Disposition::Builtin,
    "recordlevellocking" => Disposition::Builtin,
    "rename" => Disposition::Builtin,
    "securityfiltering" => Disposition::Builtin,
    "setautocalcfields" => Disposition::Builtin,
    "systemcreatedatno" => Disposition::Builtin,
    "systemcreatedbyno" => Disposition::Builtin,
    "systemidno" => Disposition::Builtin,
    "systemmodifiedatno" => Disposition::Builtin,
    "systemmodifiedbyno" => Disposition::Builtin,
    "truncate" => Disposition::Builtin,
    "writepermission" => Disposition::Builtin,
    "setrecfilter" => Disposition::Builtin,
};

// --- FieldRef ---
static FIELDREF: phf::Set<&'static str> = phf_set! {
    "name", "number", "caption", "value", "class", "type", "relation", "active",
    "length", "optioncaption", "optionmembers", "record", "validate",
    "calcfield", "setrange", "setfilter", "getfilter", "getrangemin",
    "getrangemax", "testfield",
    // Added from compiler JSON:
    "calcsum", "enumvaluecount", "fielderror", "getenumvaluecaption",
    "getenumvaluecaptionfromordinalvalue", "getenumvaluename",
    "getenumvaluenamefromordinalvalue", "getenumvalueordinal", "isenum",
    "isoptimizedfortextsearch", "optionstring",
};

// --- KeyRef ---
static KEYREF: phf::Set<&'static str> = phf_set! {
    "fieldcount", "fieldindex", "active", "ascending",
};

// --- Json* ---
static JSONOBJECT: phf::Set<&'static str> = phf_set! {
    "add", "contains", "get", "remove", "replace", "keys", "values",
    "selecttoken", "readfrom", "writeto", "astoken", "getenumerator", "path",
    "gettype",
    // Added from compiler JSON:
    "clone", "getarray", "getbiginteger", "getboolean", "getbyte", "getchar",
    "getdate", "getdatetime", "getdecimal", "getduration", "getinteger",
    "getobject", "getoption", "gettext", "gettime", "readfromyaml",
    "selecttokens", "writewithsecretsto", "writetoyaml",
};
static JSONTOKEN: phf::Set<&'static str> = phf_set! {
    "isarray", "isobject", "isvalue", "asarray", "asobject", "asvalue",
    "selecttoken", "readfrom", "writeto", "gettype", "path", "clone",
    "writelinesto",
    // Added from compiler JSON:
    "selecttokens",
};
static JSONARRAY: phf::Set<&'static str> = phf_set! {
    "add", "addfirst", "set", "get", "getrange", "remove", "indexof", "contains",
    "count", "readfrom", "writeto", "astoken", "getenumerator", "path",
    "gettype", "insert",
    // Added from compiler JSON:
    "clone", "getarray", "getbiginteger", "getboolean", "getbyte", "getchar",
    "getdate", "getdatetime", "getdecimal", "getduration", "getinteger",
    "getobject", "getoption", "gettext", "gettime", "removeat", "selecttoken",
    "selecttokens",
};
static JSONVALUE: phf::Set<&'static str> = phf_set! {
    "asboolean", "asbyte", "asinteger", "asbiginteger", "asdecimal", "asoption",
    "astext", "aschar", "ascode", "asdate", "astime", "asdatetime", "asduration",
    "asguid", "setvalue", "readfrom", "writeto", "isnull", "isundefined",
    "gettype", "clone",
    // Added from compiler JSON:
    "astoken", "path", "selecttoken", "setvaluetonull", "setvaluetoundefined",
};

// --- Http* ---
static HTTPCLIENT: phf::Set<&'static str> = phf_set! {
    "get", "post", "put", "patch", "delete", "send", "clear", "addrequestheader",
    "defaultrequestheaders", "timeout", "useragent", "usewindowsauthentication",
    "usedefaultnetworkwindowsauthentication",
    // Added from compiler JSON:
    "addcertificate", "getbaseaddress", "setbaseaddress", "useresponsecookies",
    "useservercertificatevalidation",
};
static HTTPREQUEST: phf::Set<&'static str> = phf_set! {
    "method", "setrequesturi", "getrequesturi", "content", "headers", "getheaders",
    // Added from compiler JSON:
    "getcookie", "getcookienames", "getsecretrequesturi", "removecookie",
    "setcookie", "setsecretrequesturi",
};
static HTTPRESPONSE: phf::Set<&'static str> = phf_set! {
    "issuccessstatuscode", "httpstatuscode", "reasonphrase", "content", "headers", "getheaders",
    // Added from compiler JSON:
    "getcookie", "getcookienames", "isblockedbyenvironment",
};
static HTTPHEADERS: phf::Set<&'static str> = phf_set! {
    "add", "remove", "clear", "contains", "getvalues", "tryadd",
    // Added from compiler JSON:
    "containssecret", "getsecretvalues", "keys", "tryaddwithoutvalidation",
};
static HTTPCONTENT: phf::Set<&'static str> = phf_set! {
    "writefrom", "readas", "getheaders", "clear",
};

// --- Streams ---
static INSTREAM: phf::Set<&'static str> = phf_set! {
    "read", "readtext", "eos", "len", "position", "resetposition", "readline",
    // Added from compiler JSON:
    "length",
};
static OUTSTREAM: phf::Set<&'static str> = phf_set! {
    "write", "writetext", "writeline",
};

// --- TextBuilder ---
static TEXTBUILDER: phf::Set<&'static str> = phf_set! {
    "append", "appendline", "clear", "insert", "remove", "replace", "length", "totext",
    // Added from compiler JSON:
    "capacity", "ensurecapacity", "maxcapacity",
};

// --- Dialog ---
static DIALOG: phf::Set<&'static str> = phf_set! {
    "open", "close", "update", "hidepart",
    // Added from compiler JSON:
    "confirm", "error", "hidesubsequentdialogs", "loginternalerror", "message",
    "strmenu",
};

// --- List of [T] ---
static LIST: phf::Set<&'static str> = phf_set! {
    "add", "addrange", "get", "getrange", "set", "remove", "removerange",
    "removeat", "indexof", "lastindexof", "contains", "count", "insert",
    "reverse", "getenumerator", "toarray",
};

// --- Dictionary of [K,V] ---
static DICTIONARY: phf::Set<&'static str> = phf_set! {
    "add", "set", "get", "remove", "containskey", "containsvalue", "keys",
    "values", "count", "trygetvalue", "clear",
};

// --- Xml* (one shared set across XmlDocument/Element/Node/Attribute/...). ---
static XML: phf::Set<&'static str> = phf_set! {
    "readfrom", "writeto", "create", "createelement", "createattribute",
    "createtext", "createcomment", "createdeclaration", "createcdata",
    "createprocessinginstruction", "add", "remove", "replace", "getchildnodes",
    "getchildelements", "getattributes", "getattribute", "setattribute",
    "selectsinglenode", "selectnodes", "getname", "getlocalname", "getnamespaceuri",
    "asxmlelement", "asxmlattribute", "asxmltext", "asxmlcomment", "asxmldocument",
    "isxmlelement", "isxmlattribute", "isxmltext", "isxmldocument", "isxmlnode",
    "value", "innertext", "name", "namespaceuri", "localname", "hasattributes",
    "haschildnodes", "parentnode", "firstchild", "lastchild", "nextsibling",
    "count", "get", "gettype", "clone", "normalize", "wasprocessed",
    // Added from compiler JSON (union of all Xml* types):
    "addafterself", "addbeforeself", "addfirst", "addnamespace",
    "asxmlcdata", "asxmldeclaration", "asxmldocumenttype", "asxmlnode",
    "asxmlprocessinginstruction",
    "attributes",
    "createnamespacedeclaration",
    "encoding",
    "getdata", "getdeclaration",
    "getdescendantelements", "getdescendantnodes", "getdocument",
    "getdocumenttype", "getinternalsubset", "getnamespaceofprefix",
    "getparent", "getprefixofnamespace", "getpublicid", "getroot",
    "getsystemid", "gettarget",
    "haselements", "hasnamespace",
    "innerxml", "isempty", "isnamespacedeclaration",
    "isxmlcdata", "isxmldeclaration", "isxmldocumenttype", "isxmlprocessinginstruction",
    "lookupnamespace", "lookupprefix",
    "nametable", "namespaceprefix",
    "popscope", "preservewhitespace", "pushscope",
    "removeallattributes", "removeattribute", "removenodes", "removenamespace",
    "replacenodes", "replacewith",
    "setdata", "setdeclaration", "setinternalsubset",
    "setname", "setpublicid", "setsystemid", "settarget",
    "standalone",
    "version",
};

// --- Page instance (CurrPage.M()) — methods on the current page object. ---
// Source: member_builtins.json "Page" array (19 methods), all lowercase.
static PAGE_INSTANCE: phf::Set<&'static str> = phf_set! {
    "activate",
    "cancelbackgroundtask",
    "caption",
    "close",
    "editable",
    "enqueuebackgroundtask",
    "getbackgroundparameters",
    "getrecord",
    "lookupmode",
    "objectid",
    "promptmode",
    "run",
    "runmodal",
    "saverecord",
    "setbackgroundtaskresult",
    "setrecord",
    "setselectionfilter",
    "settableview",
    "update",
};

// --- Report instance (CurrReport.M()) — methods on the current report object. ---
// Source: member_builtins.json "ReportInstance" array (36 methods), all lowercase.
static REPORT_INSTANCE: phf::Set<&'static str> = phf_set! {
    "break",
    "createtotals",
    "defaultlayout",
    "excellayout",
    "execute",
    "formatregion",
    "isreadonly",
    "language",
    "newpage",
    "newpageperrecord",
    "objectid",
    "pageno",
    "papersource",
    "preview",
    "print",
    "printonlyifdetail",
    "quit",
    "rdlclayout",
    "run",
    "runmodal",
    "runrequestpage",
    "saveas",
    "saveasexcel",
    "saveashtml",
    "saveaspdf",
    "saveasword",
    "saveasxml",
    "settableview",
    "showoutput",
    "skip",
    "targetformat",
    "totalscausedby",
    "userequestpage",
    "validateandpreparelayout",
    "wordlayout",
    "wordxmlpart",
};

// --- IsolatedStorage (static singleton) — 5 methods. ---
// Source: member_builtins.json "IsolatedStorage" array, all lowercase.
static ISOLATED_STORAGE: phf::Set<&'static str> = phf_set! {
    "contains",
    "delete",
    "get",
    "set",
    "setencrypted",
};

// --- Session (static singleton) — 19 methods. ---
// Source: member_builtins.json "Session" array, all lowercase.
static SESSION: phf::Set<&'static str> = phf_set! {
    "applicationarea",
    "applicationidentifier",
    "bindsubscription",
    "currentclienttype",
    "currentexecutionmode",
    "defaultclienttype",
    "enableverbosetelemetry",
    "getcurrentmoduleexecutioncontext",
    "getexecutioncontext",
    "getmoduleexecutioncontext",
    "issessionactive",
    "logauditmessage",
    "logmessage",
    "logsecurityaudit",
    "sendtracetag",
    "setdocumentservicetoken",
    "startsession",
    "stopsession",
    "unbindsubscription",
};

// --- NavApp (static singleton) — 16 methods. ---
// Source: member_builtins.json "NavApp" array, all lowercase.
static NAVAPP: phf::Set<&'static str> = phf_set! {
    "deletearchivedata",
    "getarchiverecordref",
    "getarchiveversion",
    "getcallercallstackmoduleinfos",
    "getcallermoduleinfo",
    "getcurrentmoduleinfo",
    "getmoduleinfo",
    "getresource",
    "getresourceasjson",
    "getresourceastext",
    "isentitled",
    "isinstalling",
    "isunlicensed",
    "listresources",
    "loadpackagedata",
    "restorearchivedata",
};

// --- TaskScheduler (static singleton) — 5 methods. ---
// Source: member_builtins.json "TaskScheduler" array, all lowercase.
static TASKSCHEDULER: phf::Set<&'static str> = phf_set! {
    "cancreatetask",
    "canceltask",
    "createtask",
    "settaskready",
    "taskexists",
};

// --- Database (static singleton) — 29 methods. ---
// Source: member_builtins.json "Database" array, all lowercase.
static DATABASE: phf::Set<&'static str> = phf_set! {
    "alterkey",
    "changeuserpassword",
    "checklicensefile",
    "commit",
    "companyname",
    "copycompany",
    "currenttransactiontype",
    "datafileinformation",
    "exportdata",
    "getdefaulttableconnection",
    "hastableconnection",
    "importdata",
    "isinwritetransaction",
    "lastusedrowversion",
    "locktimeout",
    "locktimeoutduration",
    "minimumactiverowversion",
    "registertableconnection",
    "sid",
    "selectlatestversion",
    "serialnumber",
    "serviceinstanceid",
    "sessionid",
    "setdefaulttableconnection",
    "setuserpassword",
    "tenantid",
    "unregistertableconnection",
    "userid",
    "usersecurityid",
};

// --- Blob table field — the stream-creation + value intrinsics on a `Blob` field. ---
static BLOB: phf::Set<&'static str> = phf_set! {
    "createinstream", "createoutstream", "hasvalue", "length",
};

// --- Media / MediaSet table field — media import/export/query intrinsics. ---
static MEDIA: phf::Set<&'static str> = phf_set! {
    "importfile", "importstream", "exportfile", "exportstream", "hasvalue",
    "mediaid", "count", "item", "insert", "info",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_record_and_framework_receivers() {
        assert_eq!(
            classify_receiver("Record \"Customer\""),
            Some(ReceiverBuiltinKind::Record)
        );
        assert_eq!(
            classify_receiver("Record Customer"),
            Some(ReceiverBuiltinKind::Record)
        );
        assert_eq!(
            classify_receiver("Record Item temporary"),
            Some(ReceiverBuiltinKind::Record)
        );
        assert_eq!(
            classify_receiver("RecordRef"),
            Some(ReceiverBuiltinKind::RecordRef)
        );
        assert_eq!(
            classify_receiver("FieldRef"),
            Some(ReceiverBuiltinKind::FieldRef)
        );
        assert_eq!(
            classify_receiver("KeyRef"),
            Some(ReceiverBuiltinKind::KeyRef)
        );
        assert_eq!(
            classify_receiver("JsonObject"),
            Some(ReceiverBuiltinKind::JsonObject)
        );
        assert_eq!(
            classify_receiver("List of [Text]"),
            Some(ReceiverBuiltinKind::List)
        );
        assert_eq!(
            classify_receiver("Dictionary of [Integer, Text]"),
            Some(ReceiverBuiltinKind::Dictionary)
        );
        assert_eq!(
            classify_receiver("TextBuilder"),
            Some(ReceiverBuiltinKind::TextBuilder)
        );
        assert_eq!(classify_receiver("Codeunit \"Sales-Post\""), None);
        assert_eq!(classify_receiver("Integer"), None);
        assert_eq!(classify_receiver("Text"), None);
        assert_eq!(classify_receiver(""), None);
    }

    #[test]
    fn record_builtins_hit_and_unknowns_miss() {
        for m in [
            "fieldno",
            "getview",
            "setrecfilter",
            "mark",
            "fieldcaption",
            "calcfields",
            "setrange",
            "modify",
            "findset",
        ] {
            assert_eq!(
                member_builtin_disposition(ReceiverBuiltinKind::Record, m),
                Some(Disposition::Builtin),
                "Record.{m} must be a catalog builtin"
            );
        }
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Record, "calculatediscount"),
            None
        );
    }

    #[test]
    fn recordref_flows_type_methods_are_marked() {
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::RecordRef, "open"),
            Some(Disposition::FlowsType)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::RecordRef, "gettable"),
            Some(Disposition::FlowsType)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::RecordRef, "settable"),
            Some(Disposition::FlowsType)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::RecordRef, "fieldcount"),
            Some(Disposition::Builtin)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::RecordRef, "nope"),
            None
        );
    }

    #[test]
    fn framework_builtins_hit() {
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::JsonObject, "add"),
            Some(Disposition::Builtin)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::List, "add"),
            Some(Disposition::Builtin)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Dictionary, "containskey"),
            Some(Disposition::Builtin)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::TextBuilder, "append"),
            Some(Disposition::Builtin)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Dialog, "open"),
            Some(Disposition::Builtin)
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::OutStream, "writetext"),
            Some(Disposition::Builtin)
        );
    }
}
