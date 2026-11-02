use std::path::PathBuf;

use crate::domain::workflow::{FacetKind, FacetRepository, FacetSummary, WorkflowError};

#[derive(Debug, Clone)]
pub struct WorkflowFacetFileRepository {
    base_dir: PathBuf,
}

impl WorkflowFacetFileRepository {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
}

impl FacetRepository for WorkflowFacetFileRepository {
    fn list(&self, _kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        todo!()
    }
    fn get(
        &self,
        _kind: crate::domain::workflow::FacetKind,
        _key: &str,
    ) -> Result<crate::domain::workflow::Facet, crate::domain::workflow::WorkflowError> {
        todo!()
    }
    fn get_or_new(
        &self,
        _kind: crate::domain::workflow::FacetKind,
        _key: &str,
    ) -> Result<crate::domain::workflow::Facet, crate::domain::workflow::WorkflowError> {
        todo!()
    }
    fn save(
        &self,
        _facet: crate::domain::workflow::Facet,
        _is_new: bool,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        todo!()
    }
    fn delete(
        &self,
        _facet: &crate::domain::workflow::Facet,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        todo!()
    }
    fn list_summaries(&self, _kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
        todo!()
    }
}
