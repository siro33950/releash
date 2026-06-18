use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::workflow::{
    variable_renderer, FacetKey, FacetKind, FacetRepository, WorkflowError,
};

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
        variable_renderer::render_template_variables(content, sample_values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::FacetSummary;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeFacetRepository {
        facets: Mutex<HashMap<(FacetKind, String), String>>,
    }

    impl FakeFacetRepository {
        fn get_saved(&self, kind: FacetKind, key: &str) -> Option<String> {
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

    #[test]
    fn duplicate_facet_validates_target_and_copies_source_content() {
        let facets = Arc::new(FakeFacetRepository::default());
        facets
            .save(FacetKind::Policy, "source", "policy body", true)
            .unwrap();
        let usecase = WorkflowFacetUsecase::new(facets.clone());

        usecase
            .duplicate_facet(FacetKind::Policy, "source", "copy")
            .unwrap();

        assert_eq!(
            facets.get_saved(FacetKind::Policy, "copy").as_deref(),
            Some("policy body")
        );
        assert!(usecase
            .duplicate_facet(FacetKind::Policy, "source", "../bad")
            .is_err());
    }

    #[test]
    fn render_facet_preview_delegates_to_domain_variable_renderer() {
        let facets = Arc::new(FakeFacetRepository::default());
        let usecase = WorkflowFacetUsecase::new(facets);
        let values = HashMap::from([("task".to_string(), "write tests".to_string())]);

        assert_eq!(
            usecase.render_facet_preview("Task: {{ task }}", &values),
            "Task: write tests"
        );
    }
}
