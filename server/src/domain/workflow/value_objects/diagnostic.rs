use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 診断の重大度。
pub enum Severity {
    Error,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 診断が対象とする定義の検証段階。
pub enum DiagnosticStage {
    ParseShape,
    Resolve,
    Typecheck,
    ControlFlow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// 診断の対象となるソース内の位置。
pub struct DiagnosticSpan {
    pub source: Option<String>,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, PartialEq)]
/// 定義または Facet について検出した一つの診断。
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
/// 対象ごとの重大度別の診断件数。
pub struct DiagnosticSummary {
    pub error_count: usize,
    pub info_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
/// 定義と Facet の診断および参照関係。
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
/// Facet を参照する定義、Node と枠。
pub struct FacetUsageEntry {
    pub workflow_name: String,
    pub node_name: String,
    pub slot: String,
}
