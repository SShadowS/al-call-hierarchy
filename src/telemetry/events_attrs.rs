//! Translates `EventKind` payload fields to OTel `KeyValue` attributes.
//! Kept separate from `events.rs` so the privacy lint can scan event structs
//! without crossing into pipeline code.

use crate::telemetry::events::{EventKind, ResolutionMiss};
use opentelemetry::trace::Span;
use opentelemetry::KeyValue;

pub fn apply<S: Span>(span: &mut S, event: &EventKind) {
    match event {
        EventKind::ResolutionMiss(m) => apply_resolution(span, m),
        EventKind::ParserError(e) => {
            span.set_attribute(KeyValue::new(
                "telemetry.alch.parser_kind",
                format!("{:?}", e.kind),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.file_extension",
                e.file_extension.clone(),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.file_size_bucket",
                format!("{:?}", e.file_size_bucket),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.error_count",
                e.error_count as i64,
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.repeat_count",
                e.repeat_count as i64,
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.file_hash",
                e.file_hash.clone(),
            ));
            if let Some(ref h) = e.node_kind_hash {
                span.set_attribute(KeyValue::new("telemetry.alch.node_kind_hash", h.clone()));
            }
        }
        EventKind::HandlerEmpty(h) => {
            span.set_attribute(KeyValue::new("telemetry.alch.method", h.method));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.target_object_type",
                format!("{:?}", h.target_object_type),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.target_kind",
                format!("{:?}", h.target_kind),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.object_hash",
                h.object_hash.clone(),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.procedure_hash",
                h.procedure_hash.clone(),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.repeat_count",
                h.repeat_count as i64,
            ));
        }
        EventKind::IndexerIssue(i) => {
            span.set_attribute(KeyValue::new(
                "telemetry.alch.indexer_kind",
                format!("{:?}", i.kind),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.detail_code",
                i.detail_code as i64,
            ));
            if let Some(ref h) = i.app_id_hash {
                span.set_attribute(KeyValue::new("telemetry.alch.app_id_hash", h.clone()));
            }
        }
        EventKind::SessionStart(s) => {
            span.set_attribute(KeyValue::new(
                "telemetry.alch.workspace_file_count",
                s.workspace_file_count as i64,
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.al_file_count_bucket",
                format!("{:?}", s.al_file_count_bucket),
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.dependency_count",
                s.dependency_count as i64,
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.has_app_dependencies",
                s.has_app_dependencies,
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.config_flags_bits",
                s.config_flags.bits as i64,
            ));
            span.set_attribute(KeyValue::new(
                "telemetry.alch.previous_session_unclean",
                s.previous_session_unclean,
            ));
        }
        EventKind::SessionSummary(_) => {
            // Handled in the exporter directly to avoid duplicating the attribute set.
        }
    }
}

fn apply_resolution<S: Span>(span: &mut S, m: &ResolutionMiss) {
    span.set_attribute(KeyValue::new(
        "telemetry.alch.failure",
        format!("{:?}", m.failure),
    ));
    span.set_attribute(KeyValue::new(
        "telemetry.alch.call_pattern",
        format!("{:?}", m.call_pattern),
    ));
    if let Some(t) = m.callee_object_type {
        span.set_attribute(KeyValue::new(
            "telemetry.alch.callee_object_type",
            format!("{:?}", t),
        ));
    }
    span.set_attribute(KeyValue::new(
        "telemetry.alch.callee_source",
        format!("{:?}", m.callee_source),
    ));
    span.set_attribute(KeyValue::new(
        "telemetry.alch.caller_object_type",
        format!("{:?}", m.caller_object_type),
    ));
    span.set_attribute(KeyValue::new(
        "telemetry.alch.caller_context",
        format!("{:?}", m.caller_context),
    ));
    if let Some(ref h) = m.object_hash {
        span.set_attribute(KeyValue::new("telemetry.alch.object_hash", h.clone()));
    }
    span.set_attribute(KeyValue::new(
        "telemetry.alch.procedure_hash",
        m.procedure_hash.clone(),
    ));
    span.set_attribute(KeyValue::new(
        "telemetry.alch.arg_count",
        m.arg_count as i64,
    ));
    if let Some(n) = m.name_len_object {
        span.set_attribute(KeyValue::new("telemetry.alch.name_len_object", n as i64));
    }
    span.set_attribute(KeyValue::new(
        "telemetry.alch.name_len_procedure",
        m.name_len_procedure as i64,
    ));
    span.set_attribute(KeyValue::new(
        "telemetry.alch.ts_node_path",
        m.ts_node_path.clone(),
    ));
    span.set_attribute(KeyValue::new(
        "telemetry.alch.repeat_count",
        m.repeat_count as i64,
    ));
}
