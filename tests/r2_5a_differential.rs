//! R2.5a `.app` MERGED-INDEX differential — the EXIT GATE for Phase R2.5a.
//!
//! For each committed al-sem golden under `tests/r2-5a-goldens/`, run the Rust
//! `aldump --r2.5a-merged-index` emitter (`build_merged_index_from_path`) on the
//! matching `.app` fixture(s) under `tests/r2-5a-fixtures/<fixture>/` and assert
//! the emitted projection BYTE-MATCHES the golden — over the FULL projection, not
//! just ids (Rev 2 #5: attributesParsed, object props, table fields/keys, routine
//! accessModifier, app sourceKind all byte-equal). The `.app` fixtures are the
//! EXACT SAME bytes the al-sem golden generator read (copied from al-sem's
//! committed `test/fixtures/r2.5a-deps/`), so this is a TRUE byte-parity diff.
//!
//! Default `cargo test` runs entirely OFFLINE: everything is committed in-repo.
//! A separate `#[ignore]`d `refresh_r2_5a_goldens_from_al_sem` (gated on
//! `AL_SEM_DIR`) re-copies the goldens + `.app` fixtures from an al-sem checkout.
//!
//! ## Capture point (R2.5a)
//!
//! The goldens were captured POST-`resolveModel` (which runs L3's
//! `mergeExtensionFields`), so a dep `TableExtension`'s fields are merged INTO the
//! base table. The Rust emitter reproduces that merge; the anti-degenerate matrix
//! ASSERTS the merge fires (≥1 merged extension field).
//!
//! ## Anti-degenerate matrix (fail-on-zero, [REV2] #1/#5)
//!
//! Computed from the RUST output (so it proves the emitter actually PRODUCES each
//! category, not "empty == empty"): nonzero dep objects/tables/routines; ≥1
//! internal, ≥1 local, ≥1 implementsInterfaces, ≥1 extendsTargetName, ≥1 routine
//! kind:"event-publisher", ≥1 kind:"event-subscriber", ≥1 [EventSubscriber]
//! attributesParsed arg; ≥1 entity from EACH dispatch class (ROUTINE_BEARING /
//! EXTENSION_ROUTINE_BEARING / BARE / Tables); BOTH sourceKind branches
//! (symbol-only + app-source). An oracle cross-check asserts the Rust-computed
//! matrix counts EQUAL the al-sem `manifest.json` matrix counts (ground truth).
//!
//! ## Strict comparison
//!
//! The golden and the Rust projection are compared directly: any divergence is a
//! hard failure, with no tolerance mechanism of any kind.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use al_call_hierarchy::engine::deps::merged_index::{
    build_merged_index_from_path, serialize_projection,
};
use serde_json::Value;

const R2_5A_MODEL_INSTANCE_ID: &str = "r2.5a";

/// Keys that must NEVER appear anywhere in the R2.5a projection (L4/summary/cone —
/// mirrors the al-sem `FORBIDDEN_KEYS`). HARD-FAILS, never allowlistable.
const R2_5A_FORBIDDEN_KEYS: &[&str] = &[
    "summary",
    "typedEdges",
    "intraAppCallEdges",
    "citedOperationEvidence",
    "depOrderIndex",
    "capabilityFactsDirect",
    "capabilityFactsInherited",
    "returnSummary",
    "depReturnSummaries",
    "depRoutineOrderEntries",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5a-goldens")
}

fn fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5a-fixtures")
}

/// A single divergence (golden vs rust), with a stable line-locator path.
#[derive(Debug, Clone)]
struct Divergence {
    fixture: String,
    path: String,
    golden_value: String,
    rust_value: String,
}

/// Discover every `tests/r2-5a-goldens/*.r2.5a.golden.json` (skipping manifest.json).
fn discover_goldens() -> Vec<(String, PathBuf)> {
    let dir = goldens_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".r2.5a.golden.json") {
            continue; // skips manifest.json
        }
        let fixture = name.trim_end_matches(".r2.5a.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Recursively collect every forbidden object-key in `value`, with its path.
fn scan_forbidden_keys(value: &Value, path: &str, hits: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if R2_5A_FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(child.clone());
                }
                scan_forbidden_keys(v, &child, hits);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_forbidden_keys(v, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

// --- Anti-degenerate matrices (computed from the RUST projection) -----------

const ROUTINE_BEARING_TYPES: &[&str] = &[
    "Codeunit",
    "Page",
    "Report",
    "XMLport",
    "Query",
    "Interface",
];
const EXTENSION_TYPES: &[&str] = &["TableExtension", "PageExtension"];
const BARE_TYPES: &[&str] = &[
    "Enum",
    "ControlAddIn",
    "PermissionSet",
    "PermissionSetExtension",
    "ReportExtension",
    "DotNetPackage",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DispatchMatrix {
    routine_bearing: usize,
    extension_routine_bearing: usize,
    bare: usize,
    table: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct PropertyMatrix {
    internal_routines: usize,
    local_routines: usize,
    public_routines: usize,
    event_publishers: usize,
    event_subscribers: usize,
    event_subscriber_attr_args: usize,
    objects_with_subtype: usize,
    pages_with_page_type: usize,
    pages_with_source_table: usize,
    extensions_with_target: usize,
    objects_with_interfaces: usize,
    tables_with_fields: usize,
    tables_with_keys: usize,
    keys_with2plus_fields: usize,
    attrs_with2plus_args: usize,
    routines_with2plus_params: usize,
    interfaces_implemented2plus: usize,
    /// Merged extension fields on a base table (declaringObjectId carries
    /// ":TableExtension:" but the field hangs off a non-extension table) — the
    /// capture-point invariant. A field counts when its stableFieldId's table
    /// number differs from the extension's own table number.
    merged_extension_fields: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SourceKindMatrix {
    app_source: usize,
    symbol_only: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Matrices {
    dispatch: DispatchMatrix,
    property: PropertyMatrix,
    source_kind: SourceKindMatrix,
    objects: usize,
    tables: usize,
    routines: usize,
}

impl Matrices {
    fn add(&mut self, o: &Matrices) {
        let d = &mut self.dispatch;
        d.routine_bearing += o.dispatch.routine_bearing;
        d.extension_routine_bearing += o.dispatch.extension_routine_bearing;
        d.bare += o.dispatch.bare;
        d.table += o.dispatch.table;
        let p = &mut self.property;
        p.internal_routines += o.property.internal_routines;
        p.local_routines += o.property.local_routines;
        p.public_routines += o.property.public_routines;
        p.event_publishers += o.property.event_publishers;
        p.event_subscribers += o.property.event_subscribers;
        p.event_subscriber_attr_args += o.property.event_subscriber_attr_args;
        p.objects_with_subtype += o.property.objects_with_subtype;
        p.pages_with_page_type += o.property.pages_with_page_type;
        p.pages_with_source_table += o.property.pages_with_source_table;
        p.extensions_with_target += o.property.extensions_with_target;
        p.objects_with_interfaces += o.property.objects_with_interfaces;
        p.tables_with_fields += o.property.tables_with_fields;
        p.tables_with_keys += o.property.tables_with_keys;
        p.keys_with2plus_fields += o.property.keys_with2plus_fields;
        p.attrs_with2plus_args += o.property.attrs_with2plus_args;
        p.routines_with2plus_params += o.property.routines_with2plus_params;
        p.interfaces_implemented2plus += o.property.interfaces_implemented2plus;
        p.merged_extension_fields += o.property.merged_extension_fields;
        self.source_kind.app_source += o.source_kind.app_source;
        self.source_kind.symbol_only += o.source_kind.symbol_only;
        self.objects += o.objects;
        self.tables += o.tables;
        self.routines += o.routines;
    }
}

/// Compute the matrices from one fixture's RUST projection JSON.
fn matrices_of(proj: &Value) -> Matrices {
    let mut m = Matrices::default();
    let objects = proj.get("objects").and_then(|v| v.as_array());
    let tables = proj.get("tables").and_then(|v| v.as_array());
    let routines = proj.get("routines").and_then(|v| v.as_array());
    let apps = proj.get("apps").and_then(|v| v.as_array());

    if let Some(objs) = objects {
        m.objects = objs.len();
        for o in objs {
            let ot = o.get("objectType").and_then(|v| v.as_str()).unwrap_or("");
            if ot == "Table" {
                m.dispatch.table += 1;
            } else if ROUTINE_BEARING_TYPES.contains(&ot) {
                m.dispatch.routine_bearing += 1;
            } else if EXTENSION_TYPES.contains(&ot) {
                m.dispatch.extension_routine_bearing += 1;
            } else if BARE_TYPES.contains(&ot) {
                m.dispatch.bare += 1;
            }
            if o.get("objectSubtype").is_some() {
                m.property.objects_with_subtype += 1;
            }
            if o.get("pageType").is_some() {
                m.property.pages_with_page_type += 1;
            }
            if o.get("sourceTableName").is_some() {
                m.property.pages_with_source_table += 1;
            }
            if o.get("extendsTargetName").is_some() {
                m.property.extensions_with_target += 1;
            }
            if let Some(ifaces) = o.get("implementsInterfaces").and_then(|v| v.as_array())
                && !ifaces.is_empty()
            {
                m.property.objects_with_interfaces += 1;
                if ifaces.len() >= 2 {
                    m.property.interfaces_implemented2plus += 1;
                }
            }
        }
    }

    if let Some(ts) = tables {
        m.tables = ts.len();
        for t in ts {
            let fields = t.get("fields").and_then(|v| v.as_array());
            let keys = t.get("keys").and_then(|v| v.as_array());
            if fields.map(|f| !f.is_empty()).unwrap_or(false) {
                m.property.tables_with_fields += 1;
            }
            if let Some(ks) = keys {
                if !ks.is_empty() {
                    m.property.tables_with_keys += 1;
                }
                for k in ks {
                    if k.get("fields")
                        .and_then(|v| v.as_array())
                        .map(|f| f.len() >= 2)
                        .unwrap_or(false)
                    {
                        m.property.keys_with2plus_fields += 1;
                    }
                }
            }
        }
        // Merged-extension-field detection (the capture-point invariant): a field
        // number that appears on BOTH a TableExtension's own table AND on a base
        // (non-extension) table is a field that was physically merged into the base
        // table by mergeExtensionFields. Count the base-table occurrences. Extension
        // table numbers come from the TableExtension OBJECT rows.
        let mut ext_field_keys: BTreeSet<i64> = BTreeSet::new();
        let ext_table_numbers: BTreeSet<i64> = objects
            .map(|objs| {
                objs.iter()
                    .filter(|o| {
                        o.get("objectType").and_then(|v| v.as_str()) == Some("TableExtension")
                    })
                    .filter_map(|o| o.get("objectNumber").and_then(|v| v.as_i64()))
                    .collect()
            })
            .unwrap_or_default();
        for t in ts {
            let tn = t.get("tableNumber").and_then(|v| v.as_i64()).unwrap_or(-1);
            if ext_table_numbers.contains(&tn)
                && let Some(fs) = t.get("fields").and_then(|v| v.as_array())
            {
                for f in fs {
                    if let Some(fnum) = f.get("fieldNumber").and_then(|v| v.as_i64()) {
                        ext_field_keys.insert(fnum);
                    }
                }
            }
        }
        for t in ts {
            let tn = t.get("tableNumber").and_then(|v| v.as_i64()).unwrap_or(-1);
            if ext_table_numbers.contains(&tn) {
                continue; // count merges on the BASE table only
            }
            if let Some(fs) = t.get("fields").and_then(|v| v.as_array()) {
                for f in fs {
                    if let Some(fnum) = f.get("fieldNumber").and_then(|v| v.as_i64())
                        && ext_field_keys.contains(&fnum)
                    {
                        m.property.merged_extension_fields += 1;
                    }
                }
            }
        }
    }

    if let Some(rs) = routines {
        m.routines = rs.len();
        for r in rs {
            match r.get("accessModifier").and_then(|v| v.as_str()) {
                Some("internal") => m.property.internal_routines += 1,
                Some("local") => m.property.local_routines += 1,
                _ => m.property.public_routines += 1,
            }
            match r.get("kind").and_then(|v| v.as_str()) {
                Some("event-publisher") => m.property.event_publishers += 1,
                Some("event-subscriber") => m.property.event_subscribers += 1,
                _ => {}
            }
            if r.get("parameters")
                .and_then(|v| v.as_array())
                .map(|p| p.len() >= 2)
                .unwrap_or(false)
            {
                m.property.routines_with2plus_params += 1;
            }
            if let Some(attrs) = r.get("attributesParsed").and_then(|v| v.as_array()) {
                for a in attrs {
                    let args = a.get("args").and_then(|v| v.as_array());
                    let argc = args.map(|x| x.len()).unwrap_or(0);
                    if argc >= 2 {
                        m.property.attrs_with2plus_args += 1;
                    }
                    if a.get("name")
                        .and_then(|v| v.as_str())
                        .map(|n| n.to_lowercase() == "eventsubscriber")
                        .unwrap_or(false)
                    {
                        m.property.event_subscriber_attr_args += argc;
                    }
                }
            }
        }
    }

    if let Some(aps) = apps {
        for a in aps {
            match a.get("sourceKind").and_then(|v| v.as_str()) {
                Some("app-source") => m.source_kind.app_source += 1,
                _ => m.source_kind.symbol_only += 1,
            }
        }
    }

    m
}

/// Read the al-sem manifest's oracle matrix counts.
struct ManifestOracle {
    dispatch: DispatchMatrix,
    source_kind: SourceKindMatrix,
    property: PropertyMatrix,
}

fn load_manifest_oracle() -> ManifestOracle {
    let path = goldens_dir().join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read manifest {}: {e}", path.display()));
    let m: Value = serde_json::from_str(&text).expect("manifest parses");
    let d = &m["dispatchMatrix"];
    let s = &m["sourceKindMatrix"];
    let p = &m["propertyMatrix"];
    let g = |v: &Value, k: &str| -> usize { v[k].as_u64().unwrap_or(0) as usize };
    ManifestOracle {
        dispatch: DispatchMatrix {
            routine_bearing: g(d, "routineBearing"),
            extension_routine_bearing: g(d, "extensionRoutineBearing"),
            bare: g(d, "bare"),
            table: g(d, "table"),
        },
        source_kind: SourceKindMatrix {
            app_source: g(s, "appSource"),
            symbol_only: g(s, "symbolOnly"),
        },
        property: PropertyMatrix {
            internal_routines: g(p, "internalRoutines"),
            local_routines: g(p, "localRoutines"),
            public_routines: g(p, "publicRoutines"),
            event_publishers: g(p, "eventPublishers"),
            event_subscribers: g(p, "eventSubscribers"),
            event_subscriber_attr_args: g(p, "eventSubscriberAttrArgs"),
            objects_with_subtype: g(p, "objectsWithSubtype"),
            pages_with_page_type: g(p, "pagesWithPageType"),
            pages_with_source_table: g(p, "pagesWithSourceTable"),
            extensions_with_target: g(p, "extensionsWithTarget"),
            objects_with_interfaces: g(p, "objectsWithInterfaces"),
            tables_with_fields: g(p, "tablesWithFields"),
            tables_with_keys: g(p, "tablesWithKeys"),
            keys_with2plus_fields: g(p, "keysWith2PlusFields"),
            attrs_with2plus_args: g(p, "attrsWith2PlusArgs"),
            routines_with2plus_params: g(p, "routinesWith2PlusParams"),
            interfaces_implemented2plus: g(p, "interfacesImplemented2Plus"),
            // The manifest does not carry mergedExtensionFields; the differential
            // computes + gates it directly from the rust output.
            merged_extension_fields: 0,
        },
    }
}

/// Line-by-line divergence locator between the golden + rust serialized text.
fn diff_text(fixture: &str, golden: &str, rust: &str, out: &mut Vec<Divergence>) {
    if golden == rust {
        return;
    }
    let gl: Vec<&str> = golden.lines().collect();
    let rl: Vec<&str> = rust.lines().collect();
    let n = gl.len().min(rl.len());
    for i in 0..n {
        if gl[i] != rl[i] {
            out.push(Divergence {
                fixture: fixture.to_string(),
                path: format!("line[{}]", i + 1),
                golden_value: gl[i].trim().to_string(),
                rust_value: rl[i].trim().to_string(),
            });
        }
    }
    for (i, g) in gl.iter().enumerate().skip(n) {
        out.push(Divergence {
            fixture: fixture.to_string(),
            path: format!("line[{}]:MISSING_IN_RUST", i + 1),
            golden_value: g.trim().to_string(),
            rust_value: "<absent>".to_string(),
        });
    }
    for (i, r) in rl.iter().enumerate().skip(n) {
        out.push(Divergence {
            fixture: fixture.to_string(),
            path: format!("line[{}]:EXTRA_IN_RUST", i + 1),
            golden_value: "<absent>".to_string(),
            rust_value: r.trim().to_string(),
        });
    }
}

#[test]
fn differential_r2_5a_merged_index_matches_goldens() {
    let goldens = discover_goldens();
    assert!(
        !goldens.is_empty(),
        "no R2.5a goldens discovered under {} — corpus missing?",
        goldens_dir().display()
    );

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();
    let mut totals = Matrices::default();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = fixtures_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "R2.5a golden {} has no matching `.app` fixture dir at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Rust emit (the same call `aldump --r2.5a-merged-index` makes).
        let projection = build_merged_index_from_path(&fixture_dir, R2_5A_MODEL_INSTANCE_ID);
        let rust_text = serialize_projection(&projection);
        let rust_json: Value =
            serde_json::from_str(&rust_text).expect("rust projection re-parses as JSON");

        // Golden side.
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read golden {}: {e}", golden_path.display()));
        let golden_json: Value = serde_json::from_str(&golden_text).expect("golden parses as JSON");

        // Forbidden-field scan on BOTH sides.
        scan_forbidden_keys(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_forbidden_keys(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // BYTE-level diff over the full projection (Rev 2 #5).
        diff_text(fixture, &golden_text, &rust_text, &mut all_divergences);

        // Matrices from the RUST output.
        totals.add(&matrices_of(&rust_json));
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN L4/summary/cone field(s) leaked into the R2.5a projection \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- Anti-degenerate matrix (fail-on-zero) ------------------------------
    eprintln!(
        "R2.5a matrix ({} fixture(s)): objects={} tables={} routines={} \
         dispatch(rb={} ext={} bare={} tbl={}) sourceKind(app={} sym={}) \
         events(pub={} sub={} subArgs={}) merged_ext_fields={}",
        goldens.len(),
        totals.objects,
        totals.tables,
        totals.routines,
        totals.dispatch.routine_bearing,
        totals.dispatch.extension_routine_bearing,
        totals.dispatch.bare,
        totals.dispatch.table,
        totals.source_kind.app_source,
        totals.source_kind.symbol_only,
        totals.property.event_publishers,
        totals.property.event_subscribers,
        totals.property.event_subscriber_attr_args,
        totals.property.merged_extension_fields,
    );

    let mut zero_axes: Vec<&str> = Vec::new();
    macro_rules! gate {
        ($cond:expr_2021, $name:expr_2021) => {
            if $cond {
                zero_axes.push($name);
            }
        };
    }
    gate!(totals.objects == 0, "objects");
    gate!(totals.tables == 0, "tables");
    gate!(totals.routines == 0, "routines");
    gate!(totals.property.internal_routines == 0, "internalRoutines");
    gate!(totals.property.local_routines == 0, "localRoutines");
    gate!(
        totals.property.objects_with_interfaces == 0,
        "implementsInterfaces"
    );
    gate!(
        totals.property.extensions_with_target == 0,
        "extendsTargetName"
    );
    gate!(totals.property.event_publishers == 0, "eventPublishers");
    gate!(totals.property.event_subscribers == 0, "eventSubscribers");
    gate!(
        totals.property.event_subscriber_attr_args == 0,
        "eventSubscriberAttrArgs"
    );
    gate!(
        totals.dispatch.routine_bearing == 0,
        "dispatch:ROUTINE_BEARING"
    );
    gate!(
        totals.dispatch.extension_routine_bearing == 0,
        "dispatch:EXTENSION_ROUTINE_BEARING"
    );
    gate!(totals.dispatch.bare == 0, "dispatch:BARE");
    gate!(totals.dispatch.table == 0, "dispatch:Tables");
    gate!(totals.source_kind.app_source == 0, "sourceKind:app-source");
    gate!(
        totals.source_kind.symbol_only == 0,
        "sourceKind:symbol-only"
    );
    gate!(
        totals.property.merged_extension_fields == 0,
        "mergedExtensionFields"
    );
    assert!(
        zero_axes.is_empty(),
        "DEGENERATE R2.5a matrix: axis/axes {zero_axes:?} are ZERO — the emitter is not \
         actually producing each category (empty==empty would pass a pure equality diff). \
         The matrix must prove each dispatch class / property / sourceKind / the \
         extension-field merge fires."
    );

    // --- Oracle cross-check vs the al-sem manifest matrix counts ------------
    let oracle = load_manifest_oracle();
    assert_eq!(
        totals.dispatch, oracle.dispatch,
        "R2.5a dispatch matrix MISMATCH vs al-sem manifest oracle\n  rust   = {:?}\n  oracle = {:?}",
        totals.dispatch, oracle.dispatch
    );
    assert_eq!(
        totals.source_kind, oracle.source_kind,
        "R2.5a sourceKind matrix MISMATCH vs al-sem manifest oracle\n  rust   = {:?}\n  oracle = {:?}",
        totals.source_kind, oracle.source_kind
    );
    // Property matrix sans mergedExtensionFields (not in the manifest).
    let mut rust_prop = totals.property;
    rust_prop.merged_extension_fields = 0;
    assert_eq!(
        rust_prop, oracle.property,
        "R2.5a property matrix MISMATCH vs al-sem manifest oracle\n  rust   = {:?}\n  oracle = {:?}",
        rust_prop, oracle.property
    );

    // --- Strict divergence assert --------------------------------------------
    let mut failure = String::new();
    if !all_divergences.is_empty() {
        failure.push_str(&format!(
            "\n{} R2.5a divergence(s) found:\n",
            all_divergences.len()
        ));
        for d in &all_divergences {
            failure.push_str(&format!(
                "  [{}] {}\n      golden = {}\n      rust   = {}\n",
                d.fixture, d.path, d.golden_value, d.rust_value
            ));
        }
    }

    assert!(
        failure.is_empty(),
        "R2.5a merged-index differential FAILED:{failure}"
    );

    eprintln!(
        "R2.5a differential: {} fixture(s), 0 divergences.",
        goldens.len()
    );
}

/// LIVE refresh: re-copy the goldens + `.app` fixtures from an al-sem checkout
/// (`AL_SEM_DIR`). Never runs in the normal loop. After regenerating the al-sem
/// goldens (`bun run scripts/dump-r2.5a-merged-index.ts`), this copies the
/// committed `.app` fixtures + goldens into the engine so both sides read the SAME
/// bytes. `#[ignore]`d so `cargo test` stays offline.
#[test]
#[ignore]
fn refresh_r2_5a_goldens_from_al_sem() {
    let al_sem = match std::env::var("AL_SEM_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            eprintln!("AL_SEM_DIR not set — skipping R2.5a refresh");
            return;
        }
    };
    let src_goldens = al_sem.join("scripts").join("r2.5a-goldens");
    let src_apps = al_sem.join("test").join("fixtures").join("r2.5a-deps");

    // Copy goldens (manifest + *.r2.5a.golden.json).
    let dst_goldens = goldens_dir();
    std::fs::create_dir_all(&dst_goldens).expect("mk goldens dir");
    for entry in std::fs::read_dir(&src_goldens)
        .expect("read al-sem goldens")
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".r2.5a.golden.json") || name == "manifest.json" {
            std::fs::copy(entry.path(), dst_goldens.join(&name))
                .unwrap_or_else(|e| panic!("copy golden {name}: {e}"));
        }
    }

    // Copy each `.app` fixture into its per-fixture dir (keyed via the manifest's
    // fixture → depAppGuids mapping so the layout mirrors al-sem's workspace).
    let manifest_text =
        std::fs::read_to_string(dst_goldens.join("manifest.json")).expect("read manifest");
    let manifest: Value = serde_json::from_str(&manifest_text).expect("manifest parses");
    if let Some(fixtures) = manifest["fixtures"].as_array() {
        for f in fixtures {
            let fixture = f["fixture"].as_str().unwrap();
            let dir = fixtures_dir().join(fixture);
            std::fs::create_dir_all(&dir).expect("mk fixture dir");
            if let Some(guids) = f["depAppGuids"].as_array() {
                for g in guids {
                    let guid = g.as_str().unwrap();
                    let app = format!("{guid}.app");
                    std::fs::copy(src_apps.join(&app), dir.join(&app))
                        .unwrap_or_else(|e| panic!("copy .app {app} for {fixture}: {e}"));
                }
            }
        }
    }
    eprintln!(
        "R2.5a goldens + .app fixtures refreshed from {}",
        al_sem.display()
    );
}

/// Guard: the committed `.app` fixtures must be byte-identical to al-sem's when
/// `AL_SEM_DIR` is set (proves the two sides read the SAME bytes). `#[ignore]`d so
/// the offline loop never depends on al-sem; run it after a refresh.
#[test]
#[ignore]
fn r2_5a_fixtures_match_al_sem_bytes() {
    let al_sem = match std::env::var("AL_SEM_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            eprintln!("AL_SEM_DIR not set — skipping byte-parity guard");
            return;
        }
    };
    let src_apps = al_sem.join("test").join("fixtures").join("r2.5a-deps");
    let mut checked = 0;
    fn walk(dir: &Path, src: &Path, checked: &mut usize) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, src, checked);
            } else if p.extension().and_then(|e| e.to_str()) == Some("app") {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                let ours = std::fs::read(&p).unwrap();
                let theirs = std::fs::read(src.join(&name))
                    .unwrap_or_else(|e| panic!("read al-sem .app {name}: {e}"));
                assert_eq!(
                    ours, theirs,
                    "`.app` fixture {name} differs from al-sem bytes"
                );
                *checked += 1;
            }
        }
    }
    walk(&fixtures_dir(), &src_apps, &mut checked);
    assert!(checked > 0, "no `.app` fixtures checked");
    eprintln!("R2.5a byte-parity guard: {checked} `.app` fixture(s) match al-sem.");
}
