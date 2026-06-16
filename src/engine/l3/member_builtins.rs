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
    // -----------------------------------------------------------------------
    // AL platform VALUE types — previously classified as non-object-receiver-type
    // unknowns. Source: member_builtins.json (Feature A, engine-d22).
    // -----------------------------------------------------------------------
    /// AL `Notification` — in-app notification with actions.
    /// Source: member_builtins.json "Notification" (9 methods).
    Notification,
    /// AL `ErrorInfo` — structured error information record.
    /// Source: member_builtins.json "ErrorInfo" (18 methods).
    ErrorInfo,
    /// AL `ModuleInfo` — extension metadata/dependency info.
    /// Source: member_builtins.json "ModuleInfo" (7 methods).
    ModuleInfo,
    /// AL `RecordId` — record identity handle.
    /// Source: member_builtins.json "RecordId" (2 methods).
    RecordId,
    /// AL `BigText` — large string buffer.
    /// Source: member_builtins.json "BigText" (6 methods).
    BigText,
    /// AL `SecretText` — secret / credential string.
    /// Source: member_builtins.json "SecretText" (3 methods).
    SecretText,
    /// AL `DataTransfer` — bulk data copy helper.
    /// Source: member_builtins.json "DataTransfer" (9 methods).
    DataTransfer,
    /// AL `SessionSettings` — per-session personalization settings.
    /// Source: member_builtins.json "SessionSettings" (9 methods).
    SessionSettings,
    /// AL `Text` / `Code` / `Label` string types — share the same method surface.
    /// Source: member_builtins.json "Text" (35 methods) ∪ "Label" (17 methods),
    /// deduplicated. `Code` has no separate JSON key; mapped here by convention.
    Text,
    /// AL `Date` scalar type — calendar date helpers.
    /// Source: member_builtins.json "Date" (6 methods).
    Date,
    /// AL `DateTime` scalar type — combined date+time helpers.
    /// Source: member_builtins.json "DateTime" (3 methods).
    DateTime,
    /// AL `Time` scalar type — time-of-day helpers.
    /// Source: member_builtins.json "Time" (5 methods).
    Time,
    /// AL `Guid` — globally unique identifier helpers.
    /// Source: member_builtins.json "Guid" (3 methods).
    Guid,
    /// AL `Integer` scalar — numeric helpers.
    /// Source: member_builtins.json "Integer" (1 method).
    Integer,
    /// AL `Decimal` scalar — numeric helpers.
    /// Source: member_builtins.json "Decimal" (1 method).
    Decimal,
    /// AL `Boolean` scalar — boolean helpers.
    /// Source: member_builtins.json "Boolean" (1 method).
    Boolean,
    /// AL `Duration` scalar — duration helpers.
    /// Source: member_builtins.json "Duration" (1 method).
    Duration,
    /// AL `BigInteger` scalar — big-integer helpers.
    /// Source: member_builtins.json "BigInteger" (1 method).
    BigInteger,
    /// AL `Byte` scalar — byte helpers.
    /// Source: member_builtins.json "Byte" (1 method).
    Byte,
    /// AL `File` — file I/O operations.
    /// Source: member_builtins.json "File" (28 methods).
    File,
    /// AL `FileUpload` — uploaded-file handle.
    /// Source: member_builtins.json "FileUpload" (2 methods).
    FileUpload,
    /// AL `NumberSequence` — number series helpers.
    /// Source: member_builtins.json "NumberSequence" (7 methods).
    NumberSequence,
    /// AL `Version` — semantic version helpers.
    /// Source: member_builtins.json "Version" (6 methods).
    Version,
    /// AL `FilterPageBuilder` — dynamic filter page construction.
    /// Source: member_builtins.json "FilterPageBuilder" (11 methods).
    FilterPageBuilder,
    /// AL `SessionInformation` — runtime session telemetry.
    /// Source: member_builtins.json "SessionInformation" (4 methods).
    SessionInformation,
    /// AL platform pseudo-singleton `System` — the qualified form of the global
    /// runtime functions (`System.GetCollectedErrors()`, `System.Today()`, …).
    /// Source: member_builtins.json "System" (75 methods).
    System,
    /// An AL `Enum` / `Option` VALUE (instance) — the method surface shared by every
    /// enum value: `AsInteger`, `FromInteger`, `Names`, `Ordinals`. Produced for an
    /// enum/option-typed local var and for an enum/option table FIELD used as a
    /// member receiver (`Rec."eSeal Service".Ordinals()`,
    /// `EMailTemplateLine."Mail Importance".AsInteger()`). Distinct from
    /// `ReceiverType::Enum`, which is the enum TYPE used statically.
    /// Source: member_builtins.json "EnumType" (4 methods).
    Enum,
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
    // Take the first whitespace-delimited token (handles "Record Customer", "List of [T]").
    let first = match dt.find(' ') {
        Some(i) => &dt[..i],
        None => dt,
    };
    // Also strip a length suffix like `[1024]` so `Text[1024]` and `Code[20]` normalize
    // to `text` and `code` respectively.
    let base = match first.find('[') {
        Some(i) => &first[..i],
        None => first,
    };
    let lc = base.to_lowercase();
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
        // --- Feature A: AL platform value types ---
        "notification" => ReceiverBuiltinKind::Notification,
        "errorinfo" => ReceiverBuiltinKind::ErrorInfo,
        "moduleinfo" => ReceiverBuiltinKind::ModuleInfo,
        "recordid" => ReceiverBuiltinKind::RecordId,
        "bigtext" => ReceiverBuiltinKind::BigText,
        "secrettext" => ReceiverBuiltinKind::SecretText,
        "datatransfer" => ReceiverBuiltinKind::DataTransfer,
        "sessionsettings" => ReceiverBuiltinKind::SessionSettings,
        "text" | "code" | "label" => ReceiverBuiltinKind::Text,
        "date" => ReceiverBuiltinKind::Date,
        "datetime" => ReceiverBuiltinKind::DateTime,
        "time" => ReceiverBuiltinKind::Time,
        "guid" => ReceiverBuiltinKind::Guid,
        "integer" => ReceiverBuiltinKind::Integer,
        "decimal" => ReceiverBuiltinKind::Decimal,
        "boolean" => ReceiverBuiltinKind::Boolean,
        "duration" => ReceiverBuiltinKind::Duration,
        "biginteger" => ReceiverBuiltinKind::BigInteger,
        "byte" => ReceiverBuiltinKind::Byte,
        "file" => ReceiverBuiltinKind::File,
        "fileupload" => ReceiverBuiltinKind::FileUpload,
        "numbersequence" => ReceiverBuiltinKind::NumberSequence,
        "version" => ReceiverBuiltinKind::Version,
        "filterpagebuilder" => ReceiverBuiltinKind::FilterPageBuilder,
        "sessioninformation" => ReceiverBuiltinKind::SessionInformation,
        // A variable/parameter declared `ControlAddIn "X"` — its member calls are
        // JS-side platform invocations with no in-AL target, so EVERY method is a
        // `builtin` (the same honest classification as a page UserControl receiver).
        "controladdin" => ReceiverBuiltinKind::ControlAddIn,
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
        // --- Feature A: AL platform value types ---
        Notification => set_hit(&NOTIFICATION, method_lc),
        ErrorInfo => set_hit(&ERRORINFO, method_lc),
        ModuleInfo => set_hit(&MODULEINFO, method_lc),
        RecordId => set_hit(&RECORDID, method_lc),
        BigText => set_hit(&BIGTEXT, method_lc),
        SecretText => set_hit(&SECRETTEXT, method_lc),
        DataTransfer => set_hit(&DATATRANSFER, method_lc),
        SessionSettings => set_hit(&SESSIONSETTINGS, method_lc),
        Text => set_hit(&TEXT, method_lc),
        Date => set_hit(&DATE, method_lc),
        DateTime => set_hit(&DATETIME, method_lc),
        Time => set_hit(&TIME, method_lc),
        Guid => set_hit(&GUID, method_lc),
        Integer => set_hit(&INTEGER, method_lc),
        Decimal => set_hit(&DECIMAL, method_lc),
        Boolean => set_hit(&BOOLEAN, method_lc),
        Duration => set_hit(&DURATION, method_lc),
        BigInteger => set_hit(&BIGINTEGER, method_lc),
        Byte => set_hit(&BYTE, method_lc),
        File => set_hit(&FILE, method_lc),
        FileUpload => set_hit(&FILEUPLOAD, method_lc),
        NumberSequence => set_hit(&NUMBERSEQUENCE, method_lc),
        Version => set_hit(&VERSION, method_lc),
        FilterPageBuilder => set_hit(&FILTERPAGEBUILDER, method_lc),
        SessionInformation => set_hit(&SESSIONINFORMATION, method_lc),
        Enum => set_hit(&ENUM_VALUE, method_lc),
        System => set_hit(&SYSTEM, method_lc),
    }
}

// --- System pseudo-singleton — `System.<globalfn>()`. ---
// Source: member_builtins.json "System" (75 methods).
static SYSTEM: phf::Set<&'static str> = phf_set! {
    "abs", "applicationpath", "arraylen", "calcdate", "canloadtype",
    "captionclasstranslate", "clear", "clearall", "clearcollectederrors",
    "clearlasterror", "closingdate", "codecoverageinclude", "codecoverageload",
    "codecoveragelog", "codecoveragerefresh", "compressarray", "copyarray",
    "copystream", "createdatetime", "createencryptionkey", "createguid",
    "currentdatetime", "date2dmy", "date2dwy", "dati2variant", "decrypt",
    "deleteencryptionkey", "dmy2date", "dt2date", "dt2time", "dwy2date", "encrypt",
    "encryptionenabled", "encryptionkeyexists", "evaluate", "exportencryptionkey",
    "exportobjects", "format", "getcollectederrors", "getdocumenturl",
    "getdotnettype", "getlasterrorcallstack", "getlasterrorcode",
    "getlasterrorobject", "getlasterrortext", "geturl", "globallanguage",
    "guiallowed", "hascollectederrors", "hyperlink", "importencryptionkey",
    "importobjects", "importstreamwithurlaccess", "iscollectingerrors", "isnull",
    "isnullguid", "isservicetier", "normaldate", "power", "random", "randomize",
    "round", "rounddatetime", "sleep", "temporarypath", "time", "today",
    "variant2date", "variant2time", "windowslanguage", "workdate",
};

// --- Enum / Option value instance — `<enumvalue>.AsInteger()` etc. ---
// Source: member_builtins.json "EnumType" (AsInteger, FromInteger, Names, Ordinals).
static ENUM_VALUE: phf::Set<&'static str> = phf_set! {
    "asinteger", "frominteger", "names", "ordinals",
};

#[inline]
fn set_hit(set: &phf::Set<&'static str>, method_lc: &str) -> Option<Disposition> {
    if set.contains(method_lc) {
        Some(Disposition::Builtin)
    } else {
        None
    }
}

/// The framework type RETURNED by a property access on a framework receiver —
/// `HttpClient.DefaultRequestHeaders : HttpHeaders`, `HttpResponseMessage.Content :
/// HttpContent`, etc. Enables single-hop `<fw>.<prop>.<method>()` resolution. `None`
/// when the (kind, property) pair is not a known framework-returning property.
pub fn framework_property_type(
    kind: ReceiverBuiltinKind,
    property_lc: &str,
) -> Option<ReceiverBuiltinKind> {
    use ReceiverBuiltinKind::*;
    match (kind, property_lc) {
        (HttpClient, "defaultrequestheaders") => Some(HttpHeaders),
        (HttpRequestMessage, "content") => Some(HttpContent),
        (HttpRequestMessage, "headers") => Some(HttpHeaders),
        (HttpResponseMessage, "content") => Some(HttpContent),
        (HttpResponseMessage, "headers") => Some(HttpHeaders),
        (HttpContent, "headers") => Some(HttpHeaders),
        (ErrorInfo, "customdimensions") => Some(Dictionary),
        _ => None,
    }
}

/// The framework type RETURNED by a METHOD CALL on a framework receiver —
/// `JsonToken.AsValue() : JsonValue`, `XmlNode.AsXmlElement() : Xml(Element)`,
/// `RecordRef.Field(n) : FieldRef`, etc. These are DETERMINISTIC AL framework
/// conversions (the return type never varies), so the single-hop
/// `<fw>.<method>(...).<m>()` resolution is precise. `None` when the (kind, method)
/// pair is not a known framework-returning method.
pub fn framework_method_return_type(
    kind: ReceiverBuiltinKind,
    method_lc: &str,
) -> Option<ReceiverBuiltinKind> {
    use ReceiverBuiltinKind::*;
    match (kind, method_lc) {
        // Json* conversions — `As*` always returns the corresponding Json kind.
        (JsonToken | JsonValue | JsonObject | JsonArray, "asvalue") => Some(JsonValue),
        (JsonToken | JsonValue | JsonObject | JsonArray, "asobject") => Some(JsonObject),
        (JsonToken | JsonValue | JsonObject | JsonArray, "asarray") => Some(JsonArray),
        (JsonObject | JsonArray | JsonValue, "astoken") => Some(JsonToken),
        // Xml* conversions — `As*` returns an Xml node (single shared Xml kind).
        (Xml, m)
            if matches!(
                m,
                "asxmlelement"
                    | "asxmlattribute"
                    | "asxmltext"
                    | "asxmlcomment"
                    | "asxmlcdata"
                    | "asxmldocument"
                    | "asxmlnode"
                    | "asxmldeclaration"
                    | "asxmlprocessinginstruction"
                    | "asxmldocumenttype"
            ) =>
        {
            Some(Xml)
        }
        // Xml* STATIC factory methods — `XmlElement.Create(...)`,
        // `XmlDeclaration.Create(...)`, etc. each return an Xml node (the shared Xml
        // kind), so a chained `XmlElement.Create(Name).AsXmlNode()` resolves. These
        // factories deterministically return an Xml type.
        (Xml, m)
            if matches!(
                m,
                "create"
                    | "createelement"
                    | "createattribute"
                    | "createtext"
                    | "createcomment"
                    | "createcdata"
                    | "createdeclaration"
                    | "createprocessinginstruction"
                    | "createnamespacedeclaration"
            ) =>
        {
            Some(Xml)
        }
        // Enum / Option value — `Names()` and `Ordinals()` return a List (of Text /
        // Integer respectively), so `Rec."eSeal Service".Ordinals().Count()` resolves;
        // `AsInteger()` returns Integer, so `Enum::"X"::Value.AsInteger().ToText()`
        // chains through to the Integer catalog.
        (Enum, "names") => Some(List),
        (Enum, "ordinals") => Some(List),
        (Enum, "asinteger") => Some(Integer),
        // RecordRef / KeyRef navigation.
        (RecordRef, "field") => Some(FieldRef),
        (RecordRef, "fieldindex") => Some(FieldRef),
        (RecordRef, "keyindex") => Some(KeyRef),
        (KeyRef, "fieldindex") => Some(FieldRef),
        _ => None,
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
    // Added from compiler JSON:
    "issecretcontent",
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

// =============================================================================
// Feature A: AL platform value-type catalogs
// Source: tools/gen-al-builtins/out/member_builtins.json (engine-d22)
// =============================================================================

// --- Notification — 9 methods. ---
// Source: member_builtins.json "Notification" array, all lowercase.
static NOTIFICATION: phf::Set<&'static str> = phf_set! {
    "addaction",
    "getdata",
    "hasdata",
    "id",
    "message",
    "recall",
    "scope",
    "send",
    "setdata",
};

// --- ErrorInfo — 18 methods. ---
// Source: member_builtins.json "ErrorInfo" array, all lowercase.
static ERRORINFO: phf::Set<&'static str> = phf_set! {
    "addaction",
    "addnavigationaction",
    "callstack",
    "collectible",
    "controlname",
    "create",
    "customdimensions",
    "dataclassification",
    "detailedmessage",
    "errortype",
    "fieldno",
    "message",
    "pageno",
    "recordid",
    "systemid",
    "tableid",
    "title",
    "verbosity",
};

// --- ModuleInfo — 7 methods. ---
// Source: member_builtins.json "ModuleInfo" array, all lowercase.
static MODULEINFO: phf::Set<&'static str> = phf_set! {
    "appversion",
    "dataversion",
    "dependencies",
    "id",
    "name",
    "packageid",
    "publisher",
};

// --- RecordId — 2 methods. ---
// Source: member_builtins.json "RecordId" array, all lowercase.
static RECORDID: phf::Set<&'static str> = phf_set! {
    "getrecord",
    "tableno",
};

// --- BigText — 6 methods. ---
// Source: member_builtins.json "BigText" array, all lowercase.
static BIGTEXT: phf::Set<&'static str> = phf_set! {
    "addtext",
    "getsubtext",
    "length",
    "read",
    "textpos",
    "write",
};

// --- SecretText — 3 methods. ---
// Source: member_builtins.json "SecretText" array, all lowercase.
static SECRETTEXT: phf::Set<&'static str> = phf_set! {
    "isempty",
    "secretstrsubstno",
    "unwrap",
};

// --- DataTransfer — 9 methods. ---
// Source: member_builtins.json "DataTransfer" array, all lowercase.
static DATATRANSFER: phf::Set<&'static str> = phf_set! {
    "addconstantvalue",
    "adddestinationfilter",
    "addfieldvalue",
    "addjoin",
    "addsourcefilter",
    "copyfields",
    "copyrows",
    "settables",
    "updateauditfields",
};

// --- SessionSettings — 9 methods. ---
// Source: member_builtins.json "SessionSettings" array, all lowercase.
static SESSIONSETTINGS: phf::Set<&'static str> = phf_set! {
    "company",
    "init",
    "languageid",
    "localeid",
    "profileappid",
    "profileid",
    "profilesystemscope",
    "requestsessionupdate",
    "timezone",
};

// --- Text / Code / Label — union of Text (35) and Label (17) methods, deduplicated.
// `Code` has no separate JSON key and shares the same string-method surface.
// Source: member_builtins.json "Text" + "Label" arrays, all lowercase, unioned.
static TEXT: phf::Set<&'static str> = phf_set! {
    // Text-specific methods (35)
    "contains",
    "convertstr",
    "copystr",
    "delchr",
    "delstr",
    "endswith",
    "incstr",
    "indexof",
    "indexofany",
    "insstr",
    "lastindexof",
    "lowercase",
    "maxstrlen",
    "padleft",
    "padright",
    "padstr",
    "remove",
    "replace",
    "selectstr",
    "split",
    "startswith",
    "strchecksum",
    "strlen",
    "strpos",
    "strsubstno",
    "substring",
    "tolower",
    "toupper",
    "trim",
    "trimend",
    "trimstart",
    "uppercase",
    // Label-only methods not already in Text (from "Label" JSON key)
    // (All Label methods: Contains, EndsWith, IndexOf, IndexOfAny, LastIndexOf,
    //  PadLeft, PadRight, Remove, Replace, Split, StartsWith, Substring,
    //  ToLower, ToUpper, Trim, TrimEnd, TrimStart — all already covered above)
};

// --- Date — 6 methods. ---
// Source: member_builtins.json "Date" array, all lowercase.
static DATE: phf::Set<&'static str> = phf_set! {
    "day",
    "dayofweek",
    "month",
    "totext",
    "weekno",
    "year",
};

// --- DateTime — 3 methods. ---
// Source: member_builtins.json "DateTime" array, all lowercase.
static DATETIME: phf::Set<&'static str> = phf_set! {
    "date",
    "time",
    "totext",
};

// --- Time — 5 methods. ---
// Source: member_builtins.json "Time" array, all lowercase.
static TIME: phf::Set<&'static str> = phf_set! {
    "hour",
    "millisecond",
    "minute",
    "second",
    "totext",
};

// --- Guid — 3 methods. ---
// Source: member_builtins.json "Guid" array, all lowercase.
static GUID: phf::Set<&'static str> = phf_set! {
    "createguid",
    "createsequentialguid",
    "totext",
};

// --- Integer — 1 method. ---
// Source: member_builtins.json "Integer" array, all lowercase.
static INTEGER: phf::Set<&'static str> = phf_set! {
    "totext",
};

// --- Decimal — 1 method. ---
// Source: member_builtins.json "Decimal" array, all lowercase.
static DECIMAL: phf::Set<&'static str> = phf_set! {
    "totext",
};

// --- Boolean — 1 method. ---
// Source: member_builtins.json "Boolean" array, all lowercase.
static BOOLEAN: phf::Set<&'static str> = phf_set! {
    "totext",
};

// --- Duration — 1 method. ---
// Source: member_builtins.json "Duration" array, all lowercase.
static DURATION: phf::Set<&'static str> = phf_set! {
    "totext",
};

// --- BigInteger — 1 method. ---
// Source: member_builtins.json "BigInteger" array, all lowercase.
static BIGINTEGER: phf::Set<&'static str> = phf_set! {
    "totext",
};

// --- Byte — 1 method. ---
// Source: member_builtins.json "Byte" array, all lowercase.
static BYTE: phf::Set<&'static str> = phf_set! {
    "totext",
};

// --- File — 28 methods. ---
// Source: member_builtins.json "File" array, all lowercase.
static FILE: phf::Set<&'static str> = phf_set! {
    "close",
    "copy",
    "create",
    "createinstream",
    "createoutstream",
    "createtempfile",
    "download",
    "downloadfromstream",
    "erase",
    "exists",
    "getstamp",
    "ispathtemporary",
    "len",
    "name",
    "open",
    "pos",
    "read",
    "rename",
    "seek",
    "setstamp",
    "textmode",
    "trunc",
    "upload",
    "uploadintostream",
    "view",
    "viewfromstream",
    "write",
    "writemode",
};

// --- FileUpload — 2 methods. ---
// Source: member_builtins.json "FileUpload" array, all lowercase.
static FILEUPLOAD: phf::Set<&'static str> = phf_set! {
    "createinstream",
    "filename",
};

// --- NumberSequence — 7 methods. ---
// Source: member_builtins.json "NumberSequence" array, all lowercase.
static NUMBERSEQUENCE: phf::Set<&'static str> = phf_set! {
    "current",
    "delete",
    "exists",
    "insert",
    "next",
    "range",
    "restart",
};

// --- Version — 6 methods. ---
// Source: member_builtins.json "Version" array, all lowercase.
static VERSION: phf::Set<&'static str> = phf_set! {
    "build",
    "create",
    "major",
    "minor",
    "revision",
    "totext",
};

// --- FilterPageBuilder — 11 methods. ---
// Source: member_builtins.json "FilterPageBuilder" array, all lowercase.
static FILTERPAGEBUILDER: phf::Set<&'static str> = phf_set! {
    "addfield",
    "addfieldno",
    "addrecord",
    "addrecordref",
    "addtable",
    "count",
    "getview",
    "name",
    "pagecaption",
    "runmodal",
    "setview",
};

// --- SessionInformation — 4 methods. ---
// Source: member_builtins.json "SessionInformation" array, all lowercase.
static SESSIONINFORMATION: phf::Set<&'static str> = phf_set! {
    "aitokensused",
    "callstack",
    "sqlrowsread",
    "sqlstatementsexecuted",
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
        // Integer, Text are now platform-type builtins (Feature A):
        assert_eq!(
            classify_receiver("Integer"),
            Some(ReceiverBuiltinKind::Integer)
        );
        assert_eq!(classify_receiver("Text"), Some(ReceiverBuiltinKind::Text));
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

    // --- Feature A: AL platform value-type tests ---

    #[test]
    fn platform_type_classify_receiver() {
        assert_eq!(
            classify_receiver("Notification"),
            Some(ReceiverBuiltinKind::Notification)
        );
        assert_eq!(
            classify_receiver("ErrorInfo"),
            Some(ReceiverBuiltinKind::ErrorInfo)
        );
        assert_eq!(
            classify_receiver("ModuleInfo"),
            Some(ReceiverBuiltinKind::ModuleInfo)
        );
        assert_eq!(
            classify_receiver("RecordId"),
            Some(ReceiverBuiltinKind::RecordId)
        );
        assert_eq!(
            classify_receiver("BigText"),
            Some(ReceiverBuiltinKind::BigText)
        );
        assert_eq!(
            classify_receiver("SecretText"),
            Some(ReceiverBuiltinKind::SecretText)
        );
        assert_eq!(
            classify_receiver("DataTransfer"),
            Some(ReceiverBuiltinKind::DataTransfer)
        );
        assert_eq!(
            classify_receiver("SessionSettings"),
            Some(ReceiverBuiltinKind::SessionSettings)
        );
        assert_eq!(classify_receiver("Text"), Some(ReceiverBuiltinKind::Text));
        assert_eq!(classify_receiver("Code"), Some(ReceiverBuiltinKind::Text));
        assert_eq!(classify_receiver("Label"), Some(ReceiverBuiltinKind::Text));
        assert_eq!(classify_receiver("Date"), Some(ReceiverBuiltinKind::Date));
        assert_eq!(
            classify_receiver("DateTime"),
            Some(ReceiverBuiltinKind::DateTime)
        );
        assert_eq!(classify_receiver("Time"), Some(ReceiverBuiltinKind::Time));
        assert_eq!(classify_receiver("Guid"), Some(ReceiverBuiltinKind::Guid));
        assert_eq!(
            classify_receiver("Integer"),
            Some(ReceiverBuiltinKind::Integer)
        );
        assert_eq!(
            classify_receiver("Decimal"),
            Some(ReceiverBuiltinKind::Decimal)
        );
        assert_eq!(
            classify_receiver("Boolean"),
            Some(ReceiverBuiltinKind::Boolean)
        );
        assert_eq!(
            classify_receiver("Duration"),
            Some(ReceiverBuiltinKind::Duration)
        );
        assert_eq!(
            classify_receiver("BigInteger"),
            Some(ReceiverBuiltinKind::BigInteger)
        );
        assert_eq!(classify_receiver("Byte"), Some(ReceiverBuiltinKind::Byte));
        assert_eq!(classify_receiver("File"), Some(ReceiverBuiltinKind::File));
        assert_eq!(
            classify_receiver("FileUpload"),
            Some(ReceiverBuiltinKind::FileUpload)
        );
        assert_eq!(
            classify_receiver("NumberSequence"),
            Some(ReceiverBuiltinKind::NumberSequence)
        );
        assert_eq!(
            classify_receiver("Version"),
            Some(ReceiverBuiltinKind::Version)
        );
        assert_eq!(
            classify_receiver("FilterPageBuilder"),
            Some(ReceiverBuiltinKind::FilterPageBuilder)
        );
        assert_eq!(
            classify_receiver("SessionInformation"),
            Some(ReceiverBuiltinKind::SessionInformation)
        );
    }

    #[test]
    fn length_suffixed_types_classify_correctly() {
        // Text[1024] → Text kind
        assert_eq!(
            classify_receiver("Text[1024]"),
            Some(ReceiverBuiltinKind::Text)
        );
        // Code[20] → Text kind (alias)
        assert_eq!(
            classify_receiver("Code[20]"),
            Some(ReceiverBuiltinKind::Text)
        );
        // Code[250] with trailing space shouldn't happen but is safe
        assert_eq!(
            classify_receiver("Code[250]"),
            Some(ReceiverBuiltinKind::Text)
        );
    }

    #[test]
    fn platform_type_disposition_hits() {
        // Notification.Send
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Notification, "send"),
            Some(Disposition::Builtin)
        );
        // Notification.AddAction
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Notification, "addaction"),
            Some(Disposition::Builtin)
        );
        // Text.Split
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Text, "split"),
            Some(Disposition::Builtin)
        );
        // Text.Contains
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Text, "contains"),
            Some(Disposition::Builtin)
        );
        // ErrorInfo.Message
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::ErrorInfo, "message"),
            Some(Disposition::Builtin)
        );
        // RecordId.GetRecord
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::RecordId, "getrecord"),
            Some(Disposition::Builtin)
        );
        // ModuleInfo.Publisher
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::ModuleInfo, "publisher"),
            Some(Disposition::Builtin)
        );
        // BigText.AddText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::BigText, "addtext"),
            Some(Disposition::Builtin)
        );
        // SecretText.Unwrap
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::SecretText, "unwrap"),
            Some(Disposition::Builtin)
        );
        // DataTransfer.CopyRows
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::DataTransfer, "copyrows"),
            Some(Disposition::Builtin)
        );
        // SessionSettings.Company
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::SessionSettings, "company"),
            Some(Disposition::Builtin)
        );
        // Date.DayOfWeek
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Date, "dayofweek"),
            Some(Disposition::Builtin)
        );
        // DateTime.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::DateTime, "totext"),
            Some(Disposition::Builtin)
        );
        // Time.Hour
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Time, "hour"),
            Some(Disposition::Builtin)
        );
        // Guid.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Guid, "totext"),
            Some(Disposition::Builtin)
        );
        // Integer.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Integer, "totext"),
            Some(Disposition::Builtin)
        );
        // Decimal.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Decimal, "totext"),
            Some(Disposition::Builtin)
        );
        // Boolean.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Boolean, "totext"),
            Some(Disposition::Builtin)
        );
        // Duration.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Duration, "totext"),
            Some(Disposition::Builtin)
        );
        // BigInteger.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::BigInteger, "totext"),
            Some(Disposition::Builtin)
        );
        // Byte.ToText
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Byte, "totext"),
            Some(Disposition::Builtin)
        );
        // File.Open
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::File, "open"),
            Some(Disposition::Builtin)
        );
        // FileUpload.FileName
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::FileUpload, "filename"),
            Some(Disposition::Builtin)
        );
        // NumberSequence.Next
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::NumberSequence, "next"),
            Some(Disposition::Builtin)
        );
        // Version.Major
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Version, "major"),
            Some(Disposition::Builtin)
        );
        // FilterPageBuilder.RunModal
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::FilterPageBuilder, "runmodal"),
            Some(Disposition::Builtin)
        );
        // SessionInformation.SqlRowsRead
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::SessionInformation, "sqlrowsread"),
            Some(Disposition::Builtin)
        );
    }

    #[test]
    fn platform_type_disposition_misses() {
        // Methods that don't exist on these types must return None
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Notification, "nonexistent"),
            None
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::Text, "nonexistent"),
            None
        );
        assert_eq!(
            member_builtin_disposition(ReceiverBuiltinKind::RecordId, "find"),
            None
        );
    }
}
