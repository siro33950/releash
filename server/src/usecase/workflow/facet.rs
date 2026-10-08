use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::workflow::services::template_preview;
use crate::domain::workflow::{FacetKey, FacetKind, FacetRepository, WorkflowError};

#[derive(Clone)]
pub struct WorkflowFacetUsecase {
    facets: Arc<dyn FacetRepository>,
}

impl WorkflowFacetUsecase {
    pub fn new(facets: Arc<dyn FacetRepository>) -> Self {
        Self { facets }
    }

    pub fn save_facet(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        is_new: bool,
    ) -> Result<(), WorkflowError> {
        FacetKey::new(key.to_string())?;
        self.facets.save(kind, key, content, is_new)
    }

    pub fn delete_facet(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.facets.delete(kind, key)
    }

    pub fn duplicate_facet(
        &self,
        kind: FacetKind,
        source_key: &str,
        new_key: &str,
    ) -> Result<(), WorkflowError> {
        FacetKey::new(new_key.to_string())?;
        if self.facets.list(kind)?.contains(&new_key.to_string()) {
            return Err(WorkflowError::validation(format!(
                "ファセット '{new_key}' は既に存在します"
            )));
        }
        let content = self.facets.get(kind, source_key)?;
        self.facets.save(kind, new_key, &content, true)
    }

    pub fn render_facet_preview(
        &self,
        content: &str,
        sample_values: &HashMap<String, String>,
    ) -> String {
        template_preview::render_template_variables(content, sample_values)
    }
}

#[cfg(test)]
#[path = "facet_test.rs"]
pub(crate) mod facet_tests;
