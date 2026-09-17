use super::client_handoff_query::{ClientHandoffQueryService, ClientHandoffSummary};
use crate::domain::client_operation::{
    handoff::{validate_id, validate_reference, ClientHandoffReference, ClientHandoffRepository},
    policy,
};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ClientHandoffError(String);
impl From<String> for ClientHandoffError {
    fn from(reason: String) -> Self {
        Self(reason)
    }
}
impl From<&str> for ClientHandoffError {
    fn from(reason: &str) -> Self {
        Self(reason.into())
    }
}

pub(crate) struct ClientHandoffUsecase {
    repository: Arc<dyn ClientHandoffRepository>,
    query: Arc<dyn ClientHandoffQueryService>,
}
impl ClientHandoffUsecase {
    pub fn new(
        repository: Arc<dyn ClientHandoffRepository>,
        query: Arc<dyn ClientHandoffQueryService>,
    ) -> Self {
        Self { repository, query }
    }
    pub fn remember(&self, reference: ClientHandoffReference) -> Result<(), ClientHandoffError> {
        validate_reference(
            &reference.id,
            &reference.command,
            &reference.fingerprint,
            &reference.ordering_target,
        )?;
        if !policy::persists_for_restart(&reference.command) {
            return Ok(());
        }
        self.repository
            .remember(&reference)
            .map_err(ClientHandoffError)
    }
    pub fn forget(&self, id: &str) -> Result<(), ClientHandoffError> {
        validate_id(id)?;
        self.repository.forget(id).map_err(ClientHandoffError)
    }
    pub fn list(&self) -> Result<Vec<ClientHandoffSummary>, ClientHandoffError> {
        self.query.list().map_err(ClientHandoffError)
    }
}

#[cfg(test)]
#[path = "client_handoff_test.rs"]
mod client_handoff_tests;
