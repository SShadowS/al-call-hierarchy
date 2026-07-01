//! Clean-room MEMBER-builtin catalog for Phase 3 member-call resolution.
//!
//! Sourced from `tools/gen-al-builtins/out/member_builtins.json` (AL extension
//! ms-dynamics-smb.al-18.0.2293710).  Does NOT import from
//! `crate::engine::l3::member_builtins`.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::program::resolve::member_catalog::{MemberCatalogKind, member_builtin, member_builtin_id};
//! use crate::program::resolve::receiver::FrameworkKind;
//!
//! let kind = FrameworkKind::JsonObject;
//! assert!(member_builtin(MemberCatalogKind::Framework(&kind), "add"));
//! let id = member_builtin_id(MemberCatalogKind::RecordRef, "field").unwrap();
//! assert_eq!(id.0, "RecordRef::field");
//! ```

use phf::phf_set;

use crate::program::resolve::edge::BuiltinId;
use crate::program::resolve::receiver::FrameworkKind;

// ---------------------------------------------------------------------------
// MemberCatalogKind
// ---------------------------------------------------------------------------

/// Discriminant passed to [`member_builtin`] and [`member_builtin_id`] to select
/// which catalog set to look up a method name against.
#[derive(Debug, Clone, Copy)]
pub enum MemberCatalogKind<'a> {
    RecordRef,
    FieldRef,
    KeyRef,
    Record,
    Framework(&'a FrameworkKind),
}

// ---------------------------------------------------------------------------
// Static phf sets — all lowercase, sourced from member_builtins.json
// ---------------------------------------------------------------------------

static RECORDREF: phf::Set<&'static str> = phf_set! {
    "addlink", "addloadfields", "arefieldsloaded", "ascending", "caption",
    "changecompany", "clearmarks", "close", "copy", "copylinks", "count",
    "countapprox", "currentcompany", "currentkey", "currentkeyindex", "delete",
    "deleteall", "deletelink", "deletelinks", "duplicate", "field", "fieldcount",
    "fieldexist", "fieldindex", "filtergroup", "find", "findfirst", "findlast",
    "findset", "fullyqualifiedname", "get", "getbysystemid", "getfilters",
    "getposition", "gettable", "getview", "hasfilter", "haslinks", "init",
    "insert", "isdirty", "isempty", "istemporary", "keycount", "keyindex",
    "loadfields", "locktable", "mark", "markedonly", "modify", "name", "next",
    "number", "open", "readconsistency", "readisolation", "readpermission",
    "recordid", "recordlevellocking", "rename", "reset", "securityfiltering",
    "setautocalcfields", "setloadfields", "setpermissionfilter", "setposition",
    "setrecfilter", "settable", "setview", "systemcreatedatno",
    "systemcreatedbyno", "systemidno", "systemmodifiedatno",
    "systemmodifiedbyno", "truncate", "writepermission"
};

static FIELDREF: phf::Set<&'static str> = phf_set! {
    "active", "calcfield", "calcsum", "caption", "class", "enumvaluecount",
    "fielderror", "getenumvaluecaption", "getenumvaluecaptionfromordinalvalue",
    "getenumvaluename", "getenumvaluenamefromordinalvalue",
    "getenumvalueordinal", "getfilter", "getrangemax", "getrangemin",
    "isenum", "isoptimizedfortextsearch", "length", "name", "number",
    "optioncaption", "optionmembers", "optionstring", "record", "relation",
    "setfilter", "setrange", "testfield", "type", "validate", "value"
};

static KEYREF: phf::Set<&'static str> = phf_set! {
    "active", "fieldcount", "fieldindex", "record"
};

static RECORD: phf::Set<&'static str> = phf_set! {
    "addlink", "addloadfields", "arefieldsloaded", "ascending", "calcfields",
    "calcsums", "changecompany", "clearmarks", "consistent", "copy",
    "copyfilter", "copyfilters", "copylinks", "count", "countapprox",
    "currentcompany", "currentkey", "delete", "deleteall", "deletelink",
    "deletelinks", "fieldactive", "fieldcaption", "fielderror", "fieldname",
    "fieldno", "filtergroup", "find", "findfirst", "findlast", "findset",
    "fullyqualifiedname", "get", "getascending", "getbysystemid", "getfilter",
    "getfilters", "getposition", "getrangemax", "getrangemin", "getview",
    "hasfilter", "haslinks", "init", "insert", "isempty", "istemporary",
    "loadfields", "locktable", "mark", "markedonly", "modify", "modifyall",
    "next", "readconsistency", "readisolation", "readpermission", "recordid",
    "recordlevellocking", "relation", "rename", "reset", "securityfiltering",
    "setascending", "setautocalcfields", "setbaseloadfields", "setcurrentkey",
    "setfilter", "setloadfields", "setpermissionfilter", "setposition",
    "setrange", "setrecfilter", "setview", "tablecaption", "tablename",
    "testfield", "transferfields", "truncate", "validate", "writepermission"
};

static JSONOBJECT: phf::Set<&'static str> = phf_set! {
    "add", "astoken", "clone", "contains", "get", "getarray", "getbiginteger",
    "getboolean", "getbyte", "getchar", "getdate", "getdatetime", "getdecimal",
    "getduration", "getinteger", "getobject", "getoption", "gettext", "gettime",
    "keys", "path", "readfrom", "readfromyaml", "remove", "replace",
    "selecttoken", "selecttokens", "values", "writeto", "writetoyaml",
    "writewithsecretsto"
};

static JSONTOKEN: phf::Set<&'static str> = phf_set! {
    "asarray", "asobject", "asvalue", "clone", "isarray", "isobject",
    "isvalue", "path", "readfrom", "selecttoken", "selecttokens", "writeto"
};

static JSONARRAY: phf::Set<&'static str> = phf_set! {
    "add", "astoken", "clone", "count", "get", "getarray", "getbiginteger",
    "getboolean", "getbyte", "getchar", "getdate", "getdatetime", "getdecimal",
    "getduration", "getinteger", "getobject", "getoption", "gettext", "gettime",
    "indexof", "insert", "path", "readfrom", "removeat", "selecttoken",
    "selecttokens", "set", "writeto"
};

static JSONVALUE: phf::Set<&'static str> = phf_set! {
    "asbiginteger", "asboolean", "asbyte", "aschar", "ascode", "asdate",
    "asdatetime", "asdecimal", "asduration", "asinteger", "asoption", "astext",
    "astime", "astoken", "clone", "isnull", "isundefined", "path", "readfrom",
    "selecttoken", "setvalue", "setvaluetonull", "setvaluetoundefined", "writeto"
};

static HTTPCLIENT: phf::Set<&'static str> = phf_set! {
    "addcertificate", "clear", "defaultrequestheaders", "delete", "get",
    "getbaseaddress", "patch", "post", "put", "send", "setbaseaddress",
    "timeout", "usedefaultnetworkwindowsauthentication", "useresponsecookies",
    "useservercertificatevalidation", "usewindowsauthentication"
};

static HTTPREQUEST: phf::Set<&'static str> = phf_set! {
    "content", "getcookie", "getcookienames", "getheaders", "getrequesturi",
    "getsecretrequesturi", "method", "removecookie", "setcookie",
    "setrequesturi", "setsecretrequesturi"
};

static HTTPRESPONSE: phf::Set<&'static str> = phf_set! {
    "content", "getcookie", "getcookienames", "headers", "httpstatuscode",
    "isblockedbyenvironment", "issuccessstatuscode", "reasonphrase"
};

static HTTPCONTENT: phf::Set<&'static str> = phf_set! {
    "clear", "getheaders", "issecretcontent", "readas", "writefrom"
};

static HTTPHEADERS: phf::Set<&'static str> = phf_set! {
    "add", "clear", "contains", "containssecret", "getsecretvalues",
    "getvalues", "keys", "remove", "tryaddwithoutvalidation"
};

static INSTREAM: phf::Set<&'static str> = phf_set! {
    "eos", "length", "position", "read", "readtext", "resetposition"
};

static OUTSTREAM: phf::Set<&'static str> = phf_set! {
    "write", "writetext"
};

static TEXTBUILDER: phf::Set<&'static str> = phf_set! {
    "append", "appendline", "capacity", "clear", "ensurecapacity", "insert",
    "length", "maxcapacity", "remove", "replace", "totext"
};

static TEXT: phf::Set<&'static str> = phf_set! {
    "contains", "convertstr", "copystr", "delchr", "delstr", "endswith",
    "incstr", "indexof", "indexofany", "insstr", "lastindexof", "lowercase",
    "maxstrlen", "padleft", "padright", "padstr", "remove", "replace",
    "selectstr", "split", "startswith", "strchecksum", "strlen", "strpos",
    "strsubstno", "substring", "tolower", "toupper", "trim", "trimend",
    "trimstart", "uppercase"
};

static BIGTEXT: phf::Set<&'static str> = phf_set! {
    "addtext", "getsubtext", "length", "read", "textpos", "write"
};

static SECRETTEXT: phf::Set<&'static str> = phf_set! {
    "isempty", "secretstrsubstno", "unwrap"
};

static LIST: phf::Set<&'static str> = phf_set! {
    "add", "addrange", "contains", "count", "get", "getrange", "indexof",
    "insert", "lastindexof", "remove", "removeat", "removerange", "reverse",
    "set"
};

static DICTIONARY: phf::Set<&'static str> = phf_set! {
    "add", "containskey", "count", "get", "keys", "remove", "set", "values"
};

static XML: phf::Set<&'static str> = phf_set! {
    "add", "addafterself", "addbeforeself", "addfirst", "addnamespace",
    "asxmlattribute", "asxmlcdata", "asxmlcomment", "asxmldeclaration",
    "asxmldocument", "asxmldocumenttype", "asxmlelement", "asxmlnode",
    "asxmlprocessinginstruction", "asxmltext", "attributes", "count", "create",
    "createnamespacedeclaration", "encoding", "get", "getchildelements",
    "getchildnodes", "getdata", "getdeclaration", "getdescendantelements",
    "getdescendantnodes", "getdocument", "getdocumenttype", "getinternalsubset",
    "getname", "getnamespaceofprefix", "getparent", "getprefixofnamespace",
    "getpublicid", "getroot", "getsystemid", "gettarget", "hasattributes",
    "haselements", "hasnamespace", "innertext", "innerxml", "isempty",
    "isnamespacedeclaration", "isxmlattribute", "isxmlcdata", "isxmlcomment",
    "isxmldeclaration", "isxmldocument", "isxmldocumenttype", "isxmlelement",
    "isxmlprocessinginstruction", "isxmltext", "localname", "lookupnamespace",
    "lookupprefix", "name", "nametable", "namespaceprefix", "namespaceuri",
    "popscope", "preservewhitespace", "pushscope", "readfrom", "remove",
    "removeall", "removeallattributes", "removeattribute", "removenodes",
    "removenamespace", "replacenodes", "replacewith", "selectnodes",
    "selectsinglenode", "set", "setattribute", "setdata", "setdeclaration",
    "setinternalsubset", "setname", "setpublicid", "setsystemid", "settarget",
    "standalone", "value", "version", "writeto"
};

static DATE: phf::Set<&'static str> = phf_set! {
    "day", "dayofweek", "month", "totext", "weekno", "year"
};

static DATETIME: phf::Set<&'static str> = phf_set! {
    "date", "time", "totext"
};

static TIME: phf::Set<&'static str> = phf_set! {
    "hour", "millisecond", "minute", "second", "totext"
};

static DURATION: phf::Set<&'static str> = phf_set! {
    "totext"
};

static GUID: phf::Set<&'static str> = phf_set! {
    "createguid", "createsequentialguid", "totext"
};

static BLOB: phf::Set<&'static str> = phf_set! {
    "createinstream", "createoutstream", "export", "hasvalue", "import",
    "length"
};

static MEDIA: phf::Set<&'static str> = phf_set! {
    "count", "exportfile", "exportstream", "findorphans", "hasvalue",
    "importfile", "importstream", "insert", "item", "mediaid", "remove"
};

static NOTIFICATION: phf::Set<&'static str> = phf_set! {
    "addaction", "getdata", "hasdata", "id", "message", "recall", "scope",
    "send", "setdata"
};

static ERRORINFO: phf::Set<&'static str> = phf_set! {
    "addaction", "addnavigationaction", "callstack", "collectible",
    "controlname", "create", "customdimensions", "dataclassification",
    "detailedmessage", "errortype", "fieldno", "message", "pageno", "recordid",
    "systemid", "tableid", "title", "verbosity"
};

static RECORDID: phf::Set<&'static str> = phf_set! {
    "getrecord", "tableno"
};

static MODULEINFO: phf::Set<&'static str> = phf_set! {
    "appversion", "dataversion", "dependencies", "id", "name", "packageid",
    "publisher"
};

static DATATRANSFER: phf::Set<&'static str> = phf_set! {
    "addconstantvalue", "adddestinationfilter", "addfieldvalue", "addjoin",
    "addsourcefilter", "copyfields", "copyrows", "settables", "updateauditfields"
};

static SESSIONSETTINGS: phf::Set<&'static str> = phf_set! {
    "company", "init", "languageid", "localeid", "profileappid", "profileid",
    "profilesystemscope", "requestsessionupdate", "timezone"
};

static FILTERPAGEBUILDER: phf::Set<&'static str> = phf_set! {
    "addfield", "addfieldno", "addrecord", "addrecordref", "addtable", "count",
    "getview", "name", "pagecaption", "runmodal", "setview"
};

static FILE: phf::Set<&'static str> = phf_set! {
    "close", "copy", "create", "createinstream", "createoutstream",
    "createtempfile", "download", "downloadfromstream", "erase", "exists",
    "getstamp", "ispathtemporary", "len", "name", "open", "pos", "read",
    "rename", "seek", "setstamp", "textmode", "trunc", "upload",
    "uploadintostream", "view", "viewfromstream", "write", "writemode"
};

static FILEUPLOAD: phf::Set<&'static str> = phf_set! {
    "createinstream", "filename"
};

static NUMBERSEQUENCE: phf::Set<&'static str> = phf_set! {
    "current", "delete", "exists", "insert", "next", "range", "restart"
};

static VERSION: phf::Set<&'static str> = phf_set! {
    "build", "create", "major", "minor", "revision", "totext"
};

static DIALOG: phf::Set<&'static str> = phf_set! {
    "close", "confirm", "error", "hidesubsequentdialogs", "loginternalerror",
    "message", "open", "strmenu", "update"
};

static PAGE_INSTANCE: phf::Set<&'static str> = phf_set! {
    "activate", "cancelbackgroundtask", "caption", "close", "editable",
    "enqueuebackgroundtask", "getbackgroundparameters", "getrecord",
    "lookupmode", "objectid", "promptmode", "run", "runmodal", "saverecord",
    "setbackgroundtaskresult", "setrecord", "setselectionfilter",
    "settableview", "update"
};

static REPORT_INSTANCE: phf::Set<&'static str> = phf_set! {
    "break", "createtotals", "defaultlayout", "excellayout", "execute",
    "formatregion", "isreadonly", "language", "newpage", "newpageperrecord",
    "objectid", "pageno", "papersource", "preview", "print",
    "printonlyifdetail", "quit", "rdlclayout", "run", "runmodal",
    "runrequestpage", "saveas", "saveasexcel", "saveashtml", "saveaspdf",
    "saveasword", "saveasxml", "settableview", "showoutput", "skip",
    "targetformat", "totalscausedby", "userequestpage",
    "validateandpreparelayout", "wordlayout", "wordxmlpart"
};

static SESSION: phf::Set<&'static str> = phf_set! {
    "applicationarea", "applicationidentifier", "bindsubscription",
    "currentclienttype", "currentexecutionmode", "defaultclienttype",
    "enableverbosetelemetry", "getcurrentmoduleexecutioncontext",
    "getexecutioncontext", "getmoduleexecutioncontext", "issessionactive",
    "logauditmessage", "logmessage", "logsecurityaudit", "sendtracetag",
    "setdocumentservicetoken", "startsession", "stopsession",
    "unbindsubscription"
};

static NAVAPP: phf::Set<&'static str> = phf_set! {
    "deletearchivedata", "getarchiverecordref", "getarchiveversion",
    "getcallercallstackmoduleinfos", "getcallermoduleinfo",
    "getcurrentmoduleinfo", "getmoduleinfo", "getresource",
    "getresourceasjson", "getresourceastext", "isentitled", "isinstalling",
    "isunlicensed", "listresources", "loadpackagedata", "restorearchivedata"
};

static DATABASE: phf::Set<&'static str> = phf_set! {
    "alterkey", "changeuserpassword", "checklicensefile", "commit",
    "companyname", "copycompany", "currenttransactiontype",
    "datafileinformation", "exportdata", "getdefaulttableconnection",
    "hastableconnection", "importdata", "isinwritetransaction",
    "lastusedrowversion", "locktimeout", "locktimeoutduration",
    "minimumactiverowversion", "registertableconnection", "sid",
    "selectlatestversion", "serialnumber", "serviceinstanceid", "sessionid",
    "setdefaulttableconnection", "setuserpassword", "tenantid",
    "unregistertableconnection", "userid", "usersecurityid"
};

static ISOLATED_STORAGE: phf::Set<&'static str> = phf_set! {
    "contains", "delete", "get", "set", "setencrypted"
};

static TASKSCHEDULER: phf::Set<&'static str> = phf_set! {
    "cancreatetask", "canceltask", "createtask", "settaskready", "taskexists"
};

static SYSTEM: phf::Set<&'static str> = phf_set! {
    "abs", "applicationpath", "arraylen", "calcdate", "canloadtype",
    "captionclasstranslate", "clear", "clearall", "clearcollectederrors",
    "clearlasterror", "closingdate", "codecoverageinclude", "codecoverageload",
    "codecoveragelog", "codecoveragerefresh", "compressarray", "copyarray",
    "copystream", "createdatetime", "createencryptionkey", "createguid",
    "currentdatetime", "dmy2date", "dt2date", "dt2time", "dwy2date",
    "dati2variant", "date2dmy", "date2dwy", "decrypt", "deleteencryptionkey",
    "encrypt", "encryptionenabled", "encryptionkeyexists", "evaluate",
    "exportencryptionkey", "exportobjects", "format", "getcollectederrors",
    "getdocumenturl", "getdotnettype", "getlasterrorcallstack",
    "getlasterrorcode", "getlasterrorobject", "getlasterrortext", "geturl",
    "globallanguage", "guiallowed", "hascollectederrors", "hyperlink",
    "importencryptionkey", "importobjects", "importstreamwithurlaccess",
    "iscollectingerrors", "isnull", "isnullguid", "isservicetier",
    "normaldate", "power", "random", "randomize", "round", "rounddatetime",
    "sleep", "temporarypath", "time", "today", "variant2date", "variant2time",
    "windowslanguage", "workdate"
};

static COMPANY_PROPERTY: phf::Set<&'static str> = phf_set! {
    "displayname", "id", "urlname"
};

static SESSION_INFORMATION: phf::Set<&'static str> = phf_set! {
    "aitokensused", "callstack", "sqlrowsread", "sqlstatementsexecuted"
};

static ENUM_VALUE: phf::Set<&'static str> = phf_set! {
    "asinteger", "frominteger", "names", "ordinals"
};

// ---------------------------------------------------------------------------
// Private framework lookup
// ---------------------------------------------------------------------------

/// Return `true` if `method_lc` is a known member method for `fk`.
fn framework_lookup(fk: &FrameworkKind, method_lc: &str) -> bool {
    match fk {
        FrameworkKind::JsonObject => JSONOBJECT.contains(method_lc),
        FrameworkKind::JsonToken => JSONTOKEN.contains(method_lc),
        FrameworkKind::JsonArray => JSONARRAY.contains(method_lc),
        FrameworkKind::JsonValue => JSONVALUE.contains(method_lc),
        FrameworkKind::HttpClient => HTTPCLIENT.contains(method_lc),
        FrameworkKind::HttpRequestMessage => HTTPREQUEST.contains(method_lc),
        FrameworkKind::HttpResponseMessage => HTTPRESPONSE.contains(method_lc),
        FrameworkKind::HttpContent => HTTPCONTENT.contains(method_lc),
        FrameworkKind::HttpHeaders => HTTPHEADERS.contains(method_lc),
        FrameworkKind::InStream => INSTREAM.contains(method_lc),
        FrameworkKind::OutStream => OUTSTREAM.contains(method_lc),
        FrameworkKind::TextBuilder => TEXTBUILDER.contains(method_lc),
        FrameworkKind::Text => TEXT.contains(method_lc),
        FrameworkKind::BigText => BIGTEXT.contains(method_lc),
        FrameworkKind::SecretText => SECRETTEXT.contains(method_lc),
        FrameworkKind::List => LIST.contains(method_lc),
        FrameworkKind::Dictionary => DICTIONARY.contains(method_lc),
        FrameworkKind::Xml => XML.contains(method_lc),
        FrameworkKind::Date => DATE.contains(method_lc),
        FrameworkKind::DateTime => DATETIME.contains(method_lc),
        FrameworkKind::Time => TIME.contains(method_lc),
        FrameworkKind::Duration => DURATION.contains(method_lc),
        FrameworkKind::Guid => GUID.contains(method_lc),
        FrameworkKind::Blob => BLOB.contains(method_lc),
        FrameworkKind::Media => MEDIA.contains(method_lc),
        FrameworkKind::Notification => NOTIFICATION.contains(method_lc),
        FrameworkKind::ErrorInfo => ERRORINFO.contains(method_lc),
        FrameworkKind::RecordId => RECORDID.contains(method_lc),
        FrameworkKind::ModuleInfo => MODULEINFO.contains(method_lc),
        FrameworkKind::DataTransfer => DATATRANSFER.contains(method_lc),
        FrameworkKind::SessionSettings => SESSIONSETTINGS.contains(method_lc),
        FrameworkKind::FilterPageBuilder => FILTERPAGEBUILDER.contains(method_lc),
        FrameworkKind::File => FILE.contains(method_lc),
        FrameworkKind::FileUpload => FILEUPLOAD.contains(method_lc),
        FrameworkKind::NumberSequence => NUMBERSEQUENCE.contains(method_lc),
        FrameworkKind::Version => VERSION.contains(method_lc),
        FrameworkKind::Dialog => DIALOG.contains(method_lc),
        FrameworkKind::PageInstance => PAGE_INSTANCE.contains(method_lc),
        FrameworkKind::ReportInstance => REPORT_INSTANCE.contains(method_lc),
        FrameworkKind::Session => SESSION.contains(method_lc),
        FrameworkKind::NavApp => NAVAPP.contains(method_lc),
        FrameworkKind::Database => DATABASE.contains(method_lc),
        FrameworkKind::IsolatedStorage => ISOLATED_STORAGE.contains(method_lc),
        FrameworkKind::TaskScheduler => TASKSCHEDULER.contains(method_lc),
        FrameworkKind::System => SYSTEM.contains(method_lc),
        FrameworkKind::CompanyProperty => COMPANY_PROPERTY.contains(method_lc),
        FrameworkKind::SessionInformation => SESSION_INFORMATION.contains(method_lc),
        // Every method on a ControlAddIn is a JS-side platform invocation → builtin.
        FrameworkKind::ControlAddIn => true,
        FrameworkKind::Enum => ENUM_VALUE.contains(method_lc),
        // Unknown/programmatic type — not in catalog.
        FrameworkKind::Other(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return `true` if `method_lc` (already lowercased) is a known builtin method
/// for the given `kind`.
pub fn member_builtin(kind: MemberCatalogKind<'_>, method_lc: &str) -> bool {
    match kind {
        MemberCatalogKind::RecordRef => RECORDREF.contains(method_lc),
        MemberCatalogKind::FieldRef => FIELDREF.contains(method_lc),
        MemberCatalogKind::KeyRef => KEYREF.contains(method_lc),
        MemberCatalogKind::Record => RECORD.contains(method_lc),
        MemberCatalogKind::Framework(fk) => framework_lookup(fk, method_lc),
    }
}

/// Return `Some(BuiltinId)` if `method_lc` (already lowercased) is a known
/// builtin method for the given `kind`, otherwise `None`.
///
/// The `BuiltinId` string is `"{Prefix}::{method_lc}"` where `Prefix` is:
/// - `"RecordRef"` / `"FieldRef"` / `"KeyRef"` / `"Record"` for those kinds.
/// - `format!("{fk:?}")` for `Framework(fk)` (e.g. `"JsonObject"`, `"HttpClient"`).
pub fn member_builtin_id(kind: MemberCatalogKind<'_>, method_lc: &str) -> Option<BuiltinId> {
    if !member_builtin(kind, method_lc) {
        return None;
    }
    let prefix = match kind {
        MemberCatalogKind::RecordRef => "RecordRef".to_string(),
        MemberCatalogKind::FieldRef => "FieldRef".to_string(),
        MemberCatalogKind::KeyRef => "KeyRef".to_string(),
        MemberCatalogKind::Record => "Record".to_string(),
        MemberCatalogKind::Framework(fk) => format!("{fk:?}"),
    };
    Some(BuiltinId(format!("{prefix}::{method_lc}")))
}

/// Structural (fail-closed) wrapper around [`member_builtin_id`] (beyond-1B.3b
/// Task 1 Step 4).
///
/// # Why this exists
///
/// Membership is decided by an EXACT-STRING lookup in a RECEIVER-KIND-SCOPED
/// `phf::Set` (selected by `kind` — `RECORDREF`/`FIELDREF`/`KEYREF`/`RECORD`/
/// per-`FrameworkKind`), with no hash/fingerprint digest stored or compared.
/// The id's `"{prefix}::{method_lc}"` suffix is built directly from the query,
/// so a name/receiver-kind mismatch is impossible today by construction. This
/// guard makes that invariant an executable, testable CONTRACT: it re-derives
/// the catalog hit and asserts the returned `BuiltinId`'s method-name suffix
/// equals `method_lc` before handing back a route, fail-closed (`None`) on any
/// mismatch. Callers in `resolver.rs` MUST use this (not [`member_builtin_id`]
/// directly) to classify a `Catalog` route.
pub fn member_builtin_id_checked(
    kind: MemberCatalogKind<'_>,
    method_lc: &str,
) -> Option<BuiltinId> {
    let id = member_builtin_id(kind, method_lc)?;
    let suffix = id.0.rsplit("::").next().unwrap_or(id.0.as_str());
    if suffix != method_lc {
        return None;
    }
    Some(id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_object_members_resolve() {
        let kind = FrameworkKind::JsonObject;
        assert!(member_builtin(MemberCatalogKind::Framework(&kind), "add"));
        assert!(member_builtin(MemberCatalogKind::Framework(&kind), "get"));
        assert!(member_builtin(
            MemberCatalogKind::Framework(&kind),
            "contains"
        ));
        assert!(!member_builtin(
            MemberCatalogKind::Framework(&kind),
            "notamethod"
        ));
    }

    #[test]
    fn fieldref_members_resolve() {
        assert!(member_builtin(MemberCatalogKind::FieldRef, "value"));
        assert!(member_builtin(MemberCatalogKind::FieldRef, "validate"));
        assert!(member_builtin(MemberCatalogKind::FieldRef, "name"));
        assert!(!member_builtin(MemberCatalogKind::FieldRef, "notamethod"));
    }

    #[test]
    fn httpclient_members_resolve() {
        let kind = FrameworkKind::HttpClient;
        assert!(member_builtin(MemberCatalogKind::Framework(&kind), "get"));
        assert!(member_builtin(MemberCatalogKind::Framework(&kind), "post"));
        assert!(member_builtin(MemberCatalogKind::Framework(&kind), "put"));
        assert!(!member_builtin(
            MemberCatalogKind::Framework(&kind),
            "fetch"
        ));
    }

    #[test]
    fn recordref_members_resolve() {
        assert!(member_builtin(MemberCatalogKind::RecordRef, "field"));
        assert!(member_builtin(MemberCatalogKind::RecordRef, "open"));
        assert!(member_builtin(MemberCatalogKind::RecordRef, "close"));
        assert!(!member_builtin(MemberCatalogKind::RecordRef, "notamethod"));
    }

    #[test]
    fn record_members_resolve() {
        assert!(member_builtin(MemberCatalogKind::Record, "find"));
        assert!(member_builtin(MemberCatalogKind::Record, "setrange"));
        assert!(member_builtin(MemberCatalogKind::Record, "insert"));
        assert!(!member_builtin(
            MemberCatalogKind::Record,
            "calculatediscount"
        ));
    }

    #[test]
    fn keyref_members_resolve() {
        assert!(member_builtin(MemberCatalogKind::KeyRef, "fieldcount"));
        assert!(member_builtin(MemberCatalogKind::KeyRef, "fieldindex"));
        assert!(!member_builtin(MemberCatalogKind::KeyRef, "notamethod"));
    }

    #[test]
    fn controladdin_any_method_is_builtin() {
        let kind = FrameworkKind::ControlAddIn;
        assert!(member_builtin(
            MemberCatalogKind::Framework(&kind),
            "anymethod"
        ));
        assert!(member_builtin(
            MemberCatalogKind::Framework(&kind),
            "trigger"
        ));
        assert!(member_builtin(
            MemberCatalogKind::Framework(&kind),
            "invokeextensionmethod"
        ));
    }

    #[test]
    fn other_kind_returns_false() {
        let kind = FrameworkKind::Other("unknowntype".to_string());
        assert!(!member_builtin(
            MemberCatalogKind::Framework(&kind),
            "somemethod"
        ));
    }

    #[test]
    fn member_builtin_id_format() {
        let kind = FrameworkKind::JsonObject;
        let id = member_builtin_id(MemberCatalogKind::Framework(&kind), "add").unwrap();
        assert_eq!(id.0, "JsonObject::add");

        let id2 = member_builtin_id(MemberCatalogKind::FieldRef, "value").unwrap();
        assert_eq!(id2.0, "FieldRef::value");

        let id3 = member_builtin_id(MemberCatalogKind::RecordRef, "open").unwrap();
        assert_eq!(id3.0, "RecordRef::open");

        let id4 = member_builtin_id(MemberCatalogKind::Record, "insert").unwrap();
        assert_eq!(id4.0, "Record::insert");

        assert!(member_builtin_id(MemberCatalogKind::Framework(&kind), "notamethod").is_none());
    }

    /// beyond-1B.3b Task 1 Step 4: the structural (fail-closed) wrapper agrees
    /// with the raw lookup on real hits (NAME + receiver-kind preserved in the
    /// id), and a near-miss name is correctly rejected — never classified
    /// `builtin` by coincidence. Membership is exact-string `phf::Set`
    /// containment scoped by receiver kind (no fingerprint/hash step), so a
    /// fabricated "fingerprint collision" cannot surface as a false `builtin`.
    #[test]
    fn member_builtin_id_checked_matches_raw_and_rejects_near_miss() {
        let id = member_builtin_id_checked(MemberCatalogKind::Record, "setrange").unwrap();
        assert_eq!(id.0, "Record::setrange");
        assert_eq!(
            member_builtin_id(MemberCatalogKind::Record, "setrange"),
            member_builtin_id_checked(MemberCatalogKind::Record, "setrange"),
            "checked wrapper must agree with the raw lookup on a real hit"
        );

        // Near-miss: not a real catalog member for this receiver kind.
        assert!(member_builtin_id_checked(MemberCatalogKind::Record, "setrangex").is_none());
        // Cross-kind miss: "strlen" is a Text/global method, not a Record method.
        assert!(member_builtin_id_checked(MemberCatalogKind::Record, "strlen").is_none());
    }

    #[test]
    fn coverage_guard_min_membership() {
        // Guard against accidentally empty/truncated sets after regen.
        // Key sets sizes from JSON source:
        let total = JSONOBJECT.len()
            + RECORDREF.len()
            + RECORD.len()
            + HTTPCLIENT.len()
            + XML.len()
            + FIELDREF.len();
        assert!(
            total >= 200,
            "catalog must have ≥200 entries across key sets; got {total}"
        );
    }
}
