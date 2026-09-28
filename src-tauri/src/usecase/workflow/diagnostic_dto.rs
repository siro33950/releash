use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticStage {
    ParseShape,
    Resolve,
    Typecheck,
    ControlFlow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticSpan {
    pub source: Option<String>,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticItem {
    pub code: String,
    pub severity: Severity,
    pub stage: DiagnosticStage,
    pub span: Option<DiagnosticSpan>,
    pub message: String,
    /// 対象の workflow 名（ファセット診断の場合は None）
    pub workflow_name: Option<String>,
    /// 対象の node 名
    pub node_name: Option<String>,
    /// 対象のファセットキー（ファセット診断の場合）
    pub facet_key: Option<String>,
    /// 対象のファセット種別
    pub facet_kind: Option<String>,
    /// 対象フィールド
    pub field: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiagnosticSummary {
    pub error_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticReport {
    pub items: Vec<DiagnosticItem>,
    /// workflow名 → そのworkflowの診断サマリ
    pub workflow_summaries: HashMap<String, DiagnosticSummary>,
    /// "kind/key" → そのファセットの診断サマリ
    pub facet_summaries: HashMap<String, DiagnosticSummary>,
    /// ファセットキー → 参照元 workflow/node 情報のリスト
    pub facet_usage: HashMap<String, Vec<FacetUsageEntry>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FacetUsageEntry {
    pub workflow_name: String,
    pub node_name: String,
    pub slot: String,
}
