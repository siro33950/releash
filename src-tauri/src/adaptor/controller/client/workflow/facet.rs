use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;
use crate::domain::workflow::FacetKind;
use crate::usecase::workflow::dto::{facet_summary_to_dto, FacetSummaryDto};

fn parse_domain_facet_kind(kind: &str) -> Result<FacetKind, AppError> {
    match kind {
        "policy" => Ok(FacetKind::Policy),
        "knowledge" => Ok(FacetKind::Knowledge),
        "instruction" => Ok(FacetKind::Instruction),
        _ => Err(AppError::new(format!("Unknown facet kind: {kind}"))
            .with_failure_kind(crate::domain::failure::FailureKind::InvalidInput)),
    }
}

pub(crate) async fn get_facet_shared(
    state: &AppState,
    kind: String,
    key: String,
) -> Result<String, AppError> {
    let kind = parse_domain_facet_kind(&kind)?;
    let query = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        query.get_facet(kind, &key).map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn save_facet_shared(
    state: &AppState,
    kind: String,
    key: String,
    content: String,
    is_new: Option<bool>,
) -> Result<(), AppError> {
    let kind = parse_domain_facet_kind(&kind)?;
    let is_new = is_new.unwrap_or(false);
    let usecase = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase
            .save_facet(kind, &key, &content, is_new)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn delete_facet_shared(
    state: &AppState,
    kind: String,
    key: String,
) -> Result<(), AppError> {
    let kind = parse_domain_facet_kind(&kind)?;
    let usecase = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase
            .delete_facet(kind, &key)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn list_facet_summaries_shared(
    state: &AppState,
    kind: String,
) -> Result<Vec<FacetSummaryDto>, AppError> {
    let kind = parse_domain_facet_kind(&kind)?;
    let query = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        query
            .list_facet_summaries(kind)
            .map(|summaries| summaries.into_iter().map(facet_summary_to_dto).collect())
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn duplicate_facet_shared(
    state: &AppState,
    kind: String,
    source_key: String,
    new_key: String,
) -> Result<(), AppError> {
    let kind = parse_domain_facet_kind(&kind)?;
    let usecase = state.workflow_usecase.clone();
    crate::common::operation_context::spawn_blocking(move || {
        usecase
            .duplicate_facet(kind, &source_key, &new_key)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) fn open_facet_in_editor_shared(
    state: &AppState,
    kind: String,
    key: String,
) -> Result<(), AppError> {
    let kind = parse_domain_facet_kind(&kind)?;
    state
        .workflow_usecase
        .open_facet_in_editor(kind, &key)
        .map_err(AppError::from_failure)
}
