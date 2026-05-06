//! Event structs and the canonical leaf-kind enumeration.
//!
//! See spec §5 "Data Model". 6 outer `EventKind` variants encode 14 leaf
//! event types plus a session summary. `ALL_LEAF_KINDS` is the single source
//! of truth for the count — array sizes derive from `ALL_LEAF_KINDS.len()`.

use std::time::SystemTime;

pub const SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeafKind {
    ResolutionObjectNotFound,
    ResolutionProcedureNotFound,
    ResolutionUnresolvedUnqualified,
    ResolutionAmbiguous,
    ResolutionUnsupportedConstruct,
    ParserTreeError,
    ParserParseFailed,
    ParserUnknownNodeKind,
    HandlerEmpty,
    IndexerMissingDependency,
    IndexerAppParseFailed,
    IndexerBrokenSymlink,
    IndexerIoError,
    SessionStart,
}

pub const ALL_LEAF_KINDS: [LeafKind; 14] = [
    LeafKind::ResolutionObjectNotFound,
    LeafKind::ResolutionProcedureNotFound,
    LeafKind::ResolutionUnresolvedUnqualified,
    LeafKind::ResolutionAmbiguous,
    LeafKind::ResolutionUnsupportedConstruct,
    LeafKind::ParserTreeError,
    LeafKind::ParserParseFailed,
    LeafKind::ParserUnknownNodeKind,
    LeafKind::HandlerEmpty,
    LeafKind::IndexerMissingDependency,
    LeafKind::IndexerAppParseFailed,
    LeafKind::IndexerBrokenSymlink,
    LeafKind::IndexerIoError,
    LeafKind::SessionStart,
];

impl LeafKind {
    pub fn index(self) -> usize {
        ALL_LEAF_KINDS
            .iter()
            .position(|&k| k == self)
            .expect("LeafKind must be in ALL_LEAF_KINDS")
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ResolutionObjectNotFound => "resolution.object_not_found",
            Self::ResolutionProcedureNotFound => "resolution.procedure_not_found",
            Self::ResolutionUnresolvedUnqualified => "resolution.unresolved_unqualified",
            Self::ResolutionAmbiguous => "resolution.ambiguous",
            Self::ResolutionUnsupportedConstruct => "resolution.unsupported_construct",
            Self::ParserTreeError => "parser.tree_error",
            Self::ParserParseFailed => "parser.parse_failed",
            Self::ParserUnknownNodeKind => "parser.unknown_node_kind",
            Self::HandlerEmpty => "handler.empty_result",
            Self::IndexerMissingDependency => "indexer.missing_dependency",
            Self::IndexerAppParseFailed => "indexer.app_parse_failed",
            Self::IndexerBrokenSymlink => "indexer.broken_symlink",
            Self::IndexerIoError => "indexer.io_error",
            Self::SessionStart => "session.start",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionFailure {
    ObjectNotFound,
    ProcedureNotFound,
    UnresolvedUnqualified,
    Ambiguous,
    UnsupportedConstruct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPattern {
    Qualified,
    Unqualified,
    MemberChain { depth: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Codeunit,
    Table,
    Page,
    Report,
    Query,
    XmlPort,
    Enum,
    Interface,
    PageExtension,
    TableExtension,
    EnumExtension,
    ControlAddIn,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalleeSource {
    Workspace,
    AppDependency,
    System,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerContext {
    Procedure,
    Trigger,
    EventSubscriber,
    Layout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeBucket {
    Sub1k,    // < 1KB / < 100 files
    Sub10k,   // 1-10KB / 100-500 files
    Sub100k,  // 10-100KB / 500-2000 files
    Over100k, // 100KB+ / 2000+ files
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserErrorKind {
    TreeError,
    ParseFailed,
    UnknownNodeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexerIssueKind {
    MissingDependency,
    AppParseFailed,
    BrokenSymlink,
    IoError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionKind {
    Procedure,
    Trigger,
    EventSubscriber,
}

#[derive(Debug, Clone)]
pub struct ConfigFlags {
    pub bits: u32,
}

#[derive(Debug, Clone)]
pub struct ResolutionMiss {
    pub failure: ResolutionFailure,
    pub call_pattern: CallPattern,
    pub callee_object_type: Option<ObjectType>,
    pub callee_source: CalleeSource,
    pub caller_object_type: ObjectType,
    pub caller_context: CallerContext,
    pub object_hash: Option<String>,
    pub procedure_hash: String,
    pub arg_count: u8,
    pub name_len_object: Option<u16>,
    pub name_len_procedure: u16,
    pub ts_node_path: String,
    pub repeat_count: u32,
}

#[derive(Debug, Clone)]
pub struct ParserError {
    pub kind: ParserErrorKind,
    pub node_kind_hash: Option<String>, // present for UnknownNodeKind
    pub file_hash: String,
    pub file_extension: String,
    pub file_size_bucket: SizeBucket,
    pub error_count: u32,
    pub repeat_count: u32,
}

#[derive(Debug, Clone)]
pub struct HandlerEmpty {
    pub method: &'static str,
    pub target_object_type: ObjectType,
    pub target_kind: DefinitionKind,
    pub object_hash: String,
    pub procedure_hash: String,
    pub repeat_count: u32,
}

#[derive(Debug, Clone)]
pub struct IndexerIssue {
    pub kind: IndexerIssueKind,
    pub app_id_hash: Option<String>,
    pub detail_code: u16,
}

#[derive(Debug, Clone)]
pub struct SessionStart {
    pub workspace_file_count: u32,
    pub al_file_count_bucket: SizeBucket,
    pub dependency_count: u8,
    pub has_app_dependencies: bool,
    pub config_flags: ConfigFlags,
    pub previous_session_unclean: bool,
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub duration_secs: u64,
    pub unique_patterns: u32,
    pub queue_full_drops: u32,
    pub dedup_suppressed: u32,
    pub export_attempts: u32,
    pub export_failures: u32,
    pub observed_by_kind: [u32; 14],
    pub exported_by_kind: [u32; 14],
}

#[derive(Debug, Clone)]
pub enum EventKind {
    ResolutionMiss(ResolutionMiss),
    ParserError(ParserError),
    HandlerEmpty(HandlerEmpty),
    IndexerIssue(IndexerIssue),
    SessionStart(SessionStart),
    SessionSummary(SessionSummary),
}

impl EventKind {
    /// The leaf kind for counter indexing. `SessionSummary` is meta and
    /// returns `None` (it's not self-counted).
    pub fn leaf(&self) -> Option<LeafKind> {
        match self {
            Self::ResolutionMiss(m) => Some(match m.failure {
                ResolutionFailure::ObjectNotFound => LeafKind::ResolutionObjectNotFound,
                ResolutionFailure::ProcedureNotFound => LeafKind::ResolutionProcedureNotFound,
                ResolutionFailure::UnresolvedUnqualified => {
                    LeafKind::ResolutionUnresolvedUnqualified
                }
                ResolutionFailure::Ambiguous => LeafKind::ResolutionAmbiguous,
                ResolutionFailure::UnsupportedConstruct => LeafKind::ResolutionUnsupportedConstruct,
            }),
            Self::ParserError(e) => Some(match e.kind {
                ParserErrorKind::TreeError => LeafKind::ParserTreeError,
                ParserErrorKind::ParseFailed => LeafKind::ParserParseFailed,
                ParserErrorKind::UnknownNodeKind => LeafKind::ParserUnknownNodeKind,
            }),
            Self::HandlerEmpty(_) => Some(LeafKind::HandlerEmpty),
            Self::IndexerIssue(i) => Some(match i.kind {
                IndexerIssueKind::MissingDependency => LeafKind::IndexerMissingDependency,
                IndexerIssueKind::AppParseFailed => LeafKind::IndexerAppParseFailed,
                IndexerIssueKind::BrokenSymlink => LeafKind::IndexerBrokenSymlink,
                IndexerIssueKind::IoError => LeafKind::IndexerIoError,
            }),
            Self::SessionStart(_) => Some(LeafKind::SessionStart),
            Self::SessionSummary(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EventEnvelope {
    pub schema_version: u8,
    pub timestamp: SystemTime,
    pub install_id: String,
    pub al_version: &'static str,
    pub grammar_version: &'static str,
    pub os: &'static str,
    pub session_id: u64,
    pub workspace_id: String,
    pub event: EventKind,
}

pub fn current_os() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        "linux" => "linux",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_leaf_kinds_has_14_entries() {
        assert_eq!(ALL_LEAF_KINDS.len(), 14);
    }

    #[test]
    fn each_leaf_kind_has_unique_index() {
        for (i, k) in ALL_LEAF_KINDS.iter().enumerate() {
            assert_eq!(k.index(), i);
        }
    }

    #[test]
    fn each_leaf_kind_has_unique_string() {
        let mut seen = std::collections::HashSet::new();
        for k in ALL_LEAF_KINDS {
            assert!(seen.insert(k.as_str()), "duplicate label: {}", k.as_str());
        }
    }

    #[test]
    fn session_summary_has_no_leaf() {
        let s = SessionSummary {
            duration_secs: 0,
            unique_patterns: 0,
            queue_full_drops: 0,
            dedup_suppressed: 0,
            export_attempts: 0,
            export_failures: 0,
            observed_by_kind: [0; 14],
            exported_by_kind: [0; 14],
        };
        assert_eq!(EventKind::SessionSummary(s).leaf(), None);
    }
}
