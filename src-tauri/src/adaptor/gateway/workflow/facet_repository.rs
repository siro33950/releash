use std::path::PathBuf;

use crate::adaptor::gateway::workflow::{builtin, facet as gateway_facet, prompt_rendering};
use crate::domain::workflow::{FacetKind, FacetRepository, FacetSummary, WorkflowError};

use super::mapper;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowFacetFileRepository {
    base_dir: PathBuf,
}

impl WorkflowFacetFileRepository {
    pub(crate) fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
}

impl FacetRepository for WorkflowFacetFileRepository {
    fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        gateway_facet::list_facets(mapper::domain_facet_kind_to_gateway(kind), &self.base_dir)
            .map_err(|e| WorkflowError::external(e.to_string()))
    }

    fn get(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
        gateway_facet::load_facet(
            mapper::domain_facet_kind_to_gateway(kind),
            key,
            &self.base_dir,
        )
        .map_err(|e| WorkflowError::external(e.to_string()))
    }

    fn save(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        is_new: bool,
    ) -> Result<(), WorkflowError> {
        let gateway_kind = mapper::domain_facet_kind_to_gateway(kind);
        if builtin::is_builtin_facet(gateway_kind, key) {
            return Err(WorkflowError::validation(
                "ビルトインファセットは編集できません",
            ));
        }
        let undefined = prompt_rendering::find_undefined_template_variables(content);
        if !undefined.is_empty() {
            return Err(WorkflowError::validation(format!(
                "未定義のテンプレート変数が含まれています: {}",
                undefined.join(", ")
            )));
        }
        if is_new && self.list(kind)?.contains(&key.to_string()) {
            return Err(WorkflowError::validation(format!(
                "ファセット '{key}' は既に存在します"
            )));
        }
        gateway_facet::save_facet(gateway_kind, key, content, &self.base_dir)
            .map_err(|e| WorkflowError::external(e.to_string()))
    }

    fn delete(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        gateway_facet::delete_facet(
            mapper::domain_facet_kind_to_gateway(kind),
            key,
            &self.base_dir,
        )
        .map_err(|e| WorkflowError::external(e.to_string()))
    }

    fn list_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
        gateway_facet::list_facet_summaries(
            mapper::domain_facet_kind_to_gateway(kind),
            &self.base_dir,
        )
        .map_err(|e| WorkflowError::external(e.to_string()))
        .map(|summaries| {
            summaries
                .into_iter()
                .map(mapper::gateway_facet_summary_to_domain)
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_new_rejects_duplicate_key() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());

        repo.save(FacetKind::Instruction, "impl", "body", true)
            .unwrap();
        let err = repo
            .save(FacetKind::Instruction, "impl", "body", true)
            .unwrap_err();

        assert!(matches!(err, WorkflowError::Validation(_)));
    }

    #[test]
    fn list_summaries_preserves_existing_shape_fields() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowFacetFileRepository::new(tmp.path());
        repo.save(FacetKind::Instruction, "impl", "# Title\nBody", true)
            .unwrap();

        let summaries = repo.list_summaries(FacetKind::Instruction).unwrap();

        assert!(summaries
            .iter()
            .any(|summary| summary.key == "impl" && summary.kind == "instruction"));
    }
}
