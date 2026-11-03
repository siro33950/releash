use std::path::{Path, PathBuf};

use super::{builtin, facet, storage};
use crate::domain::failure::WorkFailure;
use crate::domain::workflow::{
    FacetKey, FacetKind, WorkflowError, WorkflowExecutionSummary, WorkflowSourceFormat,
};
use crate::usecase::workflow::{
    FacetQueryService, FacetSummaryDto, WorkflowDefinitionQueryService, WorkflowSummaryDto,
};

pub struct WorkflowDefinitionFileQueryService {
    workflows_dir: PathBuf,
    facets_base_dir: PathBuf,
}

impl WorkflowDefinitionFileQueryService {
    pub fn new(workflows_dir: impl Into<PathBuf>, facets_base_dir: impl Into<PathBuf>) -> Self {
        Self {
            workflows_dir: workflows_dir.into(),
            facets_base_dir: facets_base_dir.into(),
        }
    }
}

impl WorkflowDefinitionQueryService for WorkflowDefinitionFileQueryService {
    fn list_workflows(
        &self,
        running: &[WorkflowExecutionSummary],
    ) -> Result<Vec<WorkflowSummaryDto>, WorkflowError> {
        let mut summaries = storage::workflow_files(&self.workflows_dir)?
            .into_iter()
            .map(|(name, path)| {
                let format = storage::workflow_source_format(&path).unwrap_or_default();
                let content = storage::load_workflow(&path, &self.facets_base_dir)
                    .map_err(WorkflowError::from);
                summary(name, false, format, content, running)
            })
            .collect::<Vec<_>>();
        for item in builtin::list_builtin_workflows() {
            if !summaries.iter().any(|summary| summary.name == item.name) {
                let content =
                    builtin::load_builtin_workflow_resolved(&item.name, &self.facets_base_dir)
                        .map_err(|error| WorkflowError::external(error.to_string()))
                        .and_then(|workflow| {
                            workflow.ok_or_else(|| WorkflowError::NotFound(item.name.clone()))
                        });
                summaries.push(summary(
                    item.name,
                    true,
                    WorkflowSourceFormat::Yaml,
                    content,
                    running,
                ));
            }
        }
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        let mut collapsed: Vec<WorkflowSummaryDto> = Vec::with_capacity(summaries.len());
        for summary in summaries {
            match collapsed.last_mut() {
                Some(previous) if previous.name == summary.name => {
                    previous.is_running = previous.is_running || summary.is_running;
                    previous.description = storage::DUPLICATE_NAME_DESCRIPTION.to_string();
                    if previous.failure.is_none() {
                        previous.failure = summary.failure;
                    }
                }
                _ => collapsed.push(summary),
            }
        }
        Ok(collapsed)
    }
}

fn summary(
    name: String,
    builtin: bool,
    source_format: WorkflowSourceFormat,
    content: Result<super::schema::WorkflowDefinitionYaml, WorkflowError>,
    running: &[WorkflowExecutionSummary],
) -> WorkflowSummaryDto {
    let (description, failure, execution_name) = match content {
        Ok(content) => (content.description, None, content.name),
        Err(error) => (
            if matches!(error, WorkflowError::Diagnostics(_)) {
                "Invalid workflow definition".to_string()
            } else {
                String::new()
            },
            Some(WorkFailure::from_error(&error)),
            name.clone(),
        ),
    };
    WorkflowSummaryDto {
        is_running: running
            .iter()
            .any(|item| item.workflow_name == execution_name),
        name,
        description,
        failure,
        builtin,
        source_format,
    }
}

pub struct FacetFileQueryService {
    base_dir: PathBuf,
}

impl FacetFileQueryService {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
}

impl FacetQueryService for FacetFileQueryService {
    fn list_facet_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummaryDto>, WorkflowError> {
        list_facet_summaries(kind, &self.base_dir)
            .map_err(|error| WorkflowError::external(error.to_string()))
    }
}

pub(super) fn list_facet_summaries(
    kind: FacetKind,
    base_dir: &Path,
) -> Result<Vec<FacetSummaryDto>, facet::FacetError> {
    facet::list_facets(kind, base_dir)?
        .into_iter()
        .map(|key| {
            let (description, builtin) = if FacetKey::new(key.clone()).is_ok() {
                let (source, content) = facet::load_facet_with_source(kind, &key, base_dir)?;
                let description = content
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or_default();
                (
                    description
                        .strip_prefix("# ")
                        .unwrap_or(description)
                        .trim()
                        .to_string(),
                    source.is_builtin(),
                )
            } else {
                (String::new(), false)
            };
            Ok(FacetSummaryDto {
                key,
                kind: kind.canonical_name().to_string(),
                description,
                builtin,
            })
        })
        .collect()
}
