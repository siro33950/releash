use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;
use crate::domain::workflow::FacetKind;

fn parse_domain_facet_kind(kind: &str) -> Result<FacetKind, AppError> {
    kind.parse()
        .map_err(|_| AppError::invalid_request(format!("Unknown facet kind: {kind}")))
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
