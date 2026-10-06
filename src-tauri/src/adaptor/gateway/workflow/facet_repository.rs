use std::path::PathBuf;

use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering;
use crate::adaptor::gateway::workflow::{builtin, facet as gateway_facet};
use crate::domain::workflow::{FacetKind, FacetRepository, FacetSummary, WorkflowError};

use super::mapper;

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
            .map_err(|e| WorkflowError::external(e.to_string()))?;
        if let Err(error) = super::lua::generate_editor_support(&self.base_dir) {
            log::warn!("Lua editor support generation failed after facet save: {error}");
        }
        Ok(())
    }

    fn delete(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        gateway_facet::delete_facet(
            mapper::domain_facet_kind_to_gateway(kind),
            key,
            &self.base_dir,
        )
        .map_err(|e| WorkflowError::external(e.to_string()))?;
        if let Err(error) = super::lua::generate_editor_support(&self.base_dir) {
            log::warn!("Lua editor support generation failed after facet delete: {error}");
        }
        Ok(())
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
