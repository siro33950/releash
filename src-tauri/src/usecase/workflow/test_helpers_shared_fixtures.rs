use super::*;
use crate::domain::workflow::ExecutionTreeId;
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowEventRepository};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct FakeFacetRepository {
    pub facets: Mutex<HashMap<(FacetKind, String), String>>,
}

impl FakeFacetRepository {
    pub fn get_saved(&self, kind: FacetKind, key: &str) -> Option<String> {
        self.facets
            .lock()
            .unwrap()
            .get(&(kind, key.to_string()))
            .cloned()
    }
}

impl FacetRepository for FakeFacetRepository {
    fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        Ok(self
            .facets
            .lock()
            .unwrap()
            .keys()
            .filter(|(candidate, _)| *candidate == kind)
            .map(|(_, key)| key.clone())
            .collect())
    }

    fn get(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
        self.facets
            .lock()
            .unwrap()
            .get(&(kind, key.to_string()))
            .cloned()
            .ok_or_else(|| WorkflowError::NotFound(key.to_string()))
    }

    fn save(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        _is_new: bool,
    ) -> Result<(), WorkflowError> {
        self.facets
            .lock()
            .unwrap()
            .insert((kind, key.to_string()), content.to_string());
        Ok(())
    }

    fn delete(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.facets.lock().unwrap().remove(&(kind, key.to_string()));
        Ok(())
    }

    fn list_summaries(&self, _kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub struct FakeEventRepository {
    pub events: Mutex<Vec<WorkflowEventDraft>>,
}

#[async_trait::async_trait]
impl WorkflowEventRepository for FakeEventRepository {
    #[cfg(test)]
    fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }

    async fn read(
        &self,
        _execution_id: &ExecutionTreeId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        Ok(self.events.lock().unwrap().clone())
    }
}

pub struct FakeSecretSourceGateway;

impl SecretSourceGateway for FakeSecretSourceGateway {
    fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
        Ok(vec!["token-123".to_string()])
    }
}
