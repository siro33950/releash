use std::fs;
use std::path::PathBuf;

use super::{facet as gateway_facet, storage};
use crate::domain::workflow::{
    Facet, FacetKey, FacetKind, FacetRepository, FacetSummary, WorkflowError, WorkflowSource,
};

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
    fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        gateway_facet::list_facets(kind, &self.base_dir)
            .map_err(|error| WorkflowError::external(error.to_string()))
    }

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

    fn get_or_new(&self, kind: FacetKind, key: &str) -> Result<Facet, WorkflowError> {
        match self.get(kind, key) {
            Err(WorkflowError::NotFound(_)) => Ok(Facet::restore(
                kind,
                FacetKey::new(key)?,
                self.base_dir.clone(),
                WorkflowSource::File {
                    path: self
                        .base_dir
                        .join(kind.dir_name())
                        .join(format!("{key}.md")),
                },
                String::new(),
            )),
            result => result,
        }
    }

    fn save(&self, facet: Facet, _is_new: bool) -> Result<(), WorkflowError> {
        let path = facet.file_path()?;
        storage::write_atomically(path, facet.content())?;
        if let Err(error) = super::lua::generate_editor_support(&self.base_dir) {
            log::warn!("Lua editor support generation failed after facet save: {error}");
        }
        Ok(())
    }

    fn delete(&self, facet: &Facet) -> Result<(), WorkflowError> {
        fs::remove_file(facet.file_path()?)
            .map_err(|error| WorkflowError::external(error.to_string()))?;
        if let Err(error) = super::lua::generate_editor_support(&self.base_dir) {
            log::warn!("Lua editor support generation failed after facet delete: {error}");
        }
        Ok(())
    }

    fn list_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
        gateway_facet::list_facet_summaries(kind, &self.base_dir)
            .map_err(|error| WorkflowError::external(error.to_string()))
    }
}
