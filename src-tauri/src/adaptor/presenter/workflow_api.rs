use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::usecase::workflow::{diagnostic_dto as diagnostic, dto, WorkflowEventView};

#[derive(Serialize)]
pub(crate) struct WorkflowSummaryResponse {
    name: String,
    description: String,
    builtin: bool,
    is_running: bool,
    #[serde(rename = "sourceFormat")]
    source_format: dto::WorkflowSourceFormatDto,
}

impl From<dto::WorkflowSummaryDto> for WorkflowSummaryResponse {
    fn from(value: dto::WorkflowSummaryDto) -> Self {
        Self {
            name: value.name,
            description: value.description,
            builtin: value.builtin,
            is_running: value.is_running,
            source_format: value.source_format,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowExecutionSummaryResponse {
    execution_id: String,
    workflow_name: String,
    status: &'static str,
    worktree_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_node: Option<String>,
    created_from: &'static str,
    started_at: f64,
    updated_at: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_reason: Option<String>,
    total_token_usage: TokenUsageResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenUsageResponse {
    input_tokens: u64,
    output_tokens: u64,
}

impl From<dto::WorkflowExecutionSummaryDto> for WorkflowExecutionSummaryResponse {
    fn from(value: dto::WorkflowExecutionSummaryDto) -> Self {
        Self {
            execution_id: value.execution_id,
            workflow_name: value.workflow_name,
            status: match value.status {
                dto::ExecutionStatusDto::Running => "running",
                dto::ExecutionStatusDto::Completed => "completed",
                dto::ExecutionStatusDto::Aborted => "aborted",
            },
            worktree_path: value.worktree_path,
            current_node: value.current_node,
            created_from: match value.created_from {
                dto::ExecutionOriginDto::DesktopUi => "desktop_ui",
                dto::ExecutionOriginDto::Cli => "cli",
                dto::ExecutionOriginDto::Agent => "agent",
                dto::ExecutionOriginDto::Api => "api",
            },
            started_at: value.started_at,
            updated_at: value.updated_at,
            completed_at: value.completed_at,
            error_reason: value.error_reason,
            total_token_usage: TokenUsageResponse {
                input_tokens: value.total_token_usage.input_tokens,
                output_tokens: value.total_token_usage.output_tokens,
            },
        }
    }
}

#[derive(Serialize)]
pub(crate) struct WorkflowEventResponse {
    event: String,
    execution_id: String,
    #[serde(rename = "timestampMs")]
    timestamp_ms: f64,
    #[serde(flatten)]
    payload: Map<String, Value>,
}

impl From<WorkflowEventView> for WorkflowEventResponse {
    fn from(value: WorkflowEventView) -> Self {
        Self {
            event: value.event,
            execution_id: value.execution_id,
            timestamp_ms: value.timestamp_ms,
            payload: value.payload,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SeverityResponse {
    Error,
    Info,
}

impl From<diagnostic::Severity> for SeverityResponse {
    fn from(value: diagnostic::Severity) -> Self {
        match value {
            diagnostic::Severity::Error => Self::Error,
            diagnostic::Severity::Info => Self::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticStageResponse {
    ParseShape,
    Resolve,
    Typecheck,
    ControlFlow,
}

impl From<diagnostic::DiagnosticStage> for DiagnosticStageResponse {
    fn from(value: diagnostic::DiagnosticStage) -> Self {
        match value {
            diagnostic::DiagnosticStage::ParseShape => Self::ParseShape,
            diagnostic::DiagnosticStage::Resolve => Self::Resolve,
            diagnostic::DiagnosticStage::Typecheck => Self::Typecheck,
            diagnostic::DiagnosticStage::ControlFlow => Self::ControlFlow,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct DiagnosticSpanResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    pub(crate) start_line: usize,
    pub(crate) start_col: usize,
    pub(crate) end_line: usize,
    pub(crate) end_col: usize,
}

impl From<diagnostic::DiagnosticSpan> for DiagnosticSpanResponse {
    fn from(value: diagnostic::DiagnosticSpan) -> Self {
        Self {
            source: value.source,
            start_line: value.start_line,
            start_col: value.start_col,
            end_line: value.end_line,
            end_col: value.end_col,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct DiagnosticItemResponse {
    pub(crate) code: String,
    pub(crate) severity: SeverityResponse,
    pub(crate) stage: DiagnosticStageResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) span: Option<DiagnosticSpanResponse>,
    pub(crate) message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workflow_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) node_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) facet_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) facet_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) field: Option<String>,
}

impl From<diagnostic::DiagnosticItem> for DiagnosticItemResponse {
    fn from(value: diagnostic::DiagnosticItem) -> Self {
        Self {
            code: value.code,
            severity: value.severity.into(),
            stage: value.stage.into(),
            span: value.span.map(Into::into),
            message: value.message,
            workflow_name: value.workflow_name,
            node_name: value.node_name,
            facet_key: value.facet_key,
            facet_kind: value.facet_kind,
            field: value.field,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub(crate) struct DiagnosticSummaryResponse {
    pub(crate) error_count: usize,
    pub(crate) info_count: usize,
}

impl From<diagnostic::DiagnosticSummary> for DiagnosticSummaryResponse {
    fn from(value: diagnostic::DiagnosticSummary) -> Self {
        Self {
            error_count: value.error_count,
            info_count: value.info_count,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct FacetUsageEntryResponse {
    pub(crate) workflow_name: String,
    pub(crate) node_name: String,
    pub(crate) slot: String,
}

impl From<diagnostic::FacetUsageEntry> for FacetUsageEntryResponse {
    fn from(value: diagnostic::FacetUsageEntry) -> Self {
        Self {
            workflow_name: value.workflow_name,
            node_name: value.node_name,
            slot: value.slot,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct DiagnosticReportResponse {
    pub(crate) items: Vec<DiagnosticItemResponse>,
    pub(crate) workflow_summaries: HashMap<String, DiagnosticSummaryResponse>,
    pub(crate) facet_summaries: HashMap<String, DiagnosticSummaryResponse>,
    pub(crate) facet_usage: HashMap<String, Vec<FacetUsageEntryResponse>>,
}

impl From<diagnostic::DiagnosticReport> for DiagnosticReportResponse {
    fn from(value: diagnostic::DiagnosticReport) -> Self {
        Self {
            items: value.items.into_iter().map(Into::into).collect(),
            workflow_summaries: value
                .workflow_summaries
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
            facet_summaries: value
                .facet_summaries
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect(),
            facet_usage: value
                .facet_usage
                .into_iter()
                .map(|(key, values)| (key, values.into_iter().map(Into::into).collect()))
                .collect(),
        }
    }
}

#[cfg(test)]
#[path = "workflow_api_test.rs"]
mod workflow_api_tests;
