use crate::domain::workflow::{
    FacetKind, WorkflowError, WorkflowExecutionSummary, WorkflowSourceFormat,
};

/// 定義一覧の読み取り結果。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSummaryDto {
    pub failure: Option<crate::domain::failure::WorkFailure>,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub is_running: bool,
    pub source_format: WorkflowSourceFormat,
}

/// Facet 一覧の読み取り結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetSummaryDto {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

/// 定義一覧をデータソースから読み取る。
pub trait WorkflowDefinitionQueryService: Send + Sync {
    /// 実行状況を含む定義一覧を返す。
    fn list_workflows(
        &self,
        running: &[WorkflowExecutionSummary],
    ) -> Result<Vec<WorkflowSummaryDto>, WorkflowError>;
}

/// Facet 一覧をデータソースから読み取る。
pub trait FacetQueryService: Send + Sync {
    /// 指定した種類の Facet 一覧を返す。
    fn list_facet_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummaryDto>, WorkflowError>;
}
