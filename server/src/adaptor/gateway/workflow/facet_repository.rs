use std::path::PathBuf;

use super::facet as gateway_facet;
use crate::domain::workflow::{Facet, FacetKey, FacetKind, FacetRepository, WorkflowError};

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
    fn get(&self, kind: FacetKind, key: &str) -> Result<Facet, WorkflowError> {
        let key = FacetKey::new(key)?;
        let (source, content) =
            gateway_facet::load_facet_with_source(kind, key.as_str(), &self.base_dir).map_err(
                |error| match error {
                    gateway_facet::FacetError::NotFound { key, .. } => WorkflowError::NotFound(key),
                    error => WorkflowError::external(error.to_string()),
                },
            )?;
        Ok(Facet::restore(
            kind,
            key,
            self.base_dir.clone(),
            source,
            content,
        ))
    }
}
