use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Info,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStage {
    ParseShape,
    Resolve,
    Typecheck,
    ControlFlow,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticSpan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiagnosticItem {
    pub code: String,
    pub severity: Severity,
    pub stage: DiagnosticStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<DiagnosticSpan>,
    pub message: String,
    /// 対象の workflow 名（ファセット診断の場合は None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    /// 対象の node 名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// 対象のファセットキー（ファセット診断の場合）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet_key: Option<String>,
    /// 対象のファセット種別
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet_kind: Option<String>,
    /// 対象フィールド
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct DiagnosticSummary {
    pub error_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiagnosticReport {
    pub items: Vec<DiagnosticItem>,
    /// workflow名 → そのworkflowの診断サマリ
    pub workflow_summaries: HashMap<String, DiagnosticSummary>,
    /// "kind/key" → そのファセットの診断サマリ
    pub facet_summaries: HashMap<String, DiagnosticSummary>,
    /// ファセットキー → 参照元 workflow/node 情報のリスト
    pub facet_usage: HashMap<String, Vec<FacetUsageEntry>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct FacetUsageEntry {
    pub workflow_name: String,
    pub node_name: String,
    pub slot: String,
}
