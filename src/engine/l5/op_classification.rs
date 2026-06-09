//! `classifyOp` / `isDbTouchingClass` — port of al-sem
//! `src/engine/op-classification.ts`.
//!
//! The effect class of a record operation. `touchesDb` is driven only by db-read /
//! db-write / db-lock; state-only ops feed D3's load-field analysis and
//! parameterRoles; `trigger` (Validate) has no direct DB effect — its effects
//! arrive via the Phase 2a implicit-trigger edge.
//!
//! This ports ONLY `classifyOp` + `isDbTouchingClass` (the db-effect tier the
//! d1/d2/d48/d14 path-walker detectors consume). The `RecordFlowOpRole`
//! framework (`recordFlowRoleOf`) is NOT ported here — it lands with the
//! record-flow detector wave.
//!
//! `CLASS_BY_OP` is reproduced VERBATIM from al-sem; it is total over
//! al-sem's `RecordOpType`. Op strings absent from the table return
//! [`OpEffectClass::StateOnly`] — al-sem indexes a total `Record<RecordOpType,
//! …>`, so an unknown op (one al-sem's grammar never emits) is treated as
//! state-only here rather than panicking. The native oracle pins the
//! representative ops the detectors care about.

/// The effect class of a record operation. Mirrors al-sem `OpEffectClass`
/// (`"db-read" | "db-write" | "db-lock" | "state-only" | "trigger"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpEffectClass {
    DbRead,
    DbWrite,
    DbLock,
    StateOnly,
    Trigger,
}

impl OpEffectClass {
    /// The al-sem string form (`"db-read"` etc.) — the exact class names a
    /// detector's `rootCause` / dump may byte-depend on.
    pub fn as_str(&self) -> &'static str {
        match self {
            OpEffectClass::DbRead => "db-read",
            OpEffectClass::DbWrite => "db-write",
            OpEffectClass::DbLock => "db-lock",
            OpEffectClass::StateOnly => "state-only",
            OpEffectClass::Trigger => "trigger",
        }
    }
}

/// Classify a record operation by its database effect. Pure, total over the
/// al-sem `RecordOpType` op-string set. Mirrors al-sem `classifyOp`.
///
/// The mapping reproduces al-sem's `CLASS_BY_OP` VERBATIM. Op strings not in the
/// table fall through to [`OpEffectClass::StateOnly`] (al-sem's table is total
/// over `RecordOpType`, so this only affects ops al-sem's grammar never emits).
pub fn classify_op(op: &str) -> OpEffectClass {
    match op {
        "FindSet" => OpEffectClass::DbRead,
        "FindFirst" => OpEffectClass::DbRead,
        "FindLast" => OpEffectClass::DbRead,
        "Find" => OpEffectClass::DbRead,
        "Get" => OpEffectClass::DbRead,
        "Next" => OpEffectClass::DbRead,
        "Count" => OpEffectClass::DbRead,
        "CountApprox" => OpEffectClass::DbRead,
        "IsEmpty" => OpEffectClass::DbRead,
        "CalcFields" => OpEffectClass::DbRead,
        "CalcSums" => OpEffectClass::DbRead,
        "TestField" => OpEffectClass::StateOnly,
        "Modify" => OpEffectClass::DbWrite,
        "ModifyAll" => OpEffectClass::DbWrite,
        "Insert" => OpEffectClass::DbWrite,
        "Delete" => OpEffectClass::DbWrite,
        "DeleteAll" => OpEffectClass::DbWrite,
        "LockTable" => OpEffectClass::DbLock,
        "SetLoadFields" => OpEffectClass::StateOnly,
        "AddLoadFields" => OpEffectClass::StateOnly,
        "SetRange" => OpEffectClass::StateOnly,
        "SetFilter" => OpEffectClass::StateOnly,
        "SetCurrentKey" => OpEffectClass::StateOnly,
        "Reset" => OpEffectClass::StateOnly,
        "Copy" => OpEffectClass::StateOnly,
        "TransferFields" => OpEffectClass::StateOnly,
        "Init" => OpEffectClass::StateOnly,
        "Validate" => OpEffectClass::Trigger,
        _ => OpEffectClass::StateOnly,
    }
}

/// True when this op class contributes to `touchesDb`. Mirrors al-sem
/// `isDbTouchingClass`: `db-read | db-write | db-lock`.
pub fn is_db_touching_class(cls: OpEffectClass) -> bool {
    matches!(
        cls,
        OpEffectClass::DbRead | OpEffectClass::DbWrite | OpEffectClass::DbLock
    )
}

// ===========================================================================
// Native oracles — ground-truth-free spot checks on the class table.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_class_mapping() {
        // Modify → db-write → db-touching.
        assert_eq!(classify_op("Modify"), OpEffectClass::DbWrite);
        assert!(is_db_touching_class(classify_op("Modify")));

        // Get → db-read → db-touching.
        assert_eq!(classify_op("Get"), OpEffectClass::DbRead);
        assert!(is_db_touching_class(classify_op("Get")));

        // SetRange → state-only → NOT db-touching.
        assert_eq!(classify_op("SetRange"), OpEffectClass::StateOnly);
        assert!(!is_db_touching_class(classify_op("SetRange")));

        // LockTable → db-lock → db-touching.
        assert_eq!(classify_op("LockTable"), OpEffectClass::DbLock);
        assert!(is_db_touching_class(classify_op("LockTable")));

        // Validate → trigger → NOT db-touching (effects arrive via implicit-trigger edge).
        assert_eq!(classify_op("Validate"), OpEffectClass::Trigger);
        assert!(!is_db_touching_class(classify_op("Validate")));
    }

    #[test]
    fn writes_and_reads_full_coverage() {
        for op in [
            "FindSet",
            "FindFirst",
            "FindLast",
            "Find",
            "Next",
            "Count",
            "CountApprox",
            "IsEmpty",
            "CalcFields",
            "CalcSums",
        ] {
            assert_eq!(
                classify_op(op),
                OpEffectClass::DbRead,
                "{op} should be db-read"
            );
            assert!(is_db_touching_class(classify_op(op)));
        }
        for op in ["Modify", "ModifyAll", "Insert", "Delete", "DeleteAll"] {
            assert_eq!(
                classify_op(op),
                OpEffectClass::DbWrite,
                "{op} should be db-write"
            );
            assert!(is_db_touching_class(classify_op(op)));
        }
        for op in [
            "TestField",
            "SetLoadFields",
            "AddLoadFields",
            "SetRange",
            "SetFilter",
            "SetCurrentKey",
            "Reset",
            "Copy",
            "TransferFields",
            "Init",
        ] {
            assert_eq!(
                classify_op(op),
                OpEffectClass::StateOnly,
                "{op} should be state-only"
            );
            assert!(!is_db_touching_class(classify_op(op)));
        }
    }

    #[test]
    fn class_strings_are_exact() {
        assert_eq!(OpEffectClass::DbRead.as_str(), "db-read");
        assert_eq!(OpEffectClass::DbWrite.as_str(), "db-write");
        assert_eq!(OpEffectClass::DbLock.as_str(), "db-lock");
        assert_eq!(OpEffectClass::StateOnly.as_str(), "state-only");
        assert_eq!(OpEffectClass::Trigger.as_str(), "trigger");
    }

    #[test]
    fn unknown_op_falls_through_to_state_only() {
        // An op al-sem's grammar never emits → state-only (not db-touching).
        assert_eq!(classify_op("Rename"), OpEffectClass::StateOnly);
        assert!(!is_db_touching_class(classify_op("Rename")));
    }
}
