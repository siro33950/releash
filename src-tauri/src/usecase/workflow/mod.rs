//! Workflow command usecases.
//!
//! This layer orchestrates domain aggregates through repository/gateway ports.
//! Controllers, CLI adapters, and watchers should converge
//! here as the legacy `workflow` module is removed.

pub(crate) mod approval_chat;
pub(crate) mod command;
mod definition;
pub(crate) mod dto;
pub(crate) mod event_draft;
mod facet;
mod output;
pub(crate) mod ports;
pub(crate) mod query_service;
pub(crate) mod runtime_command;
pub(crate) mod step_lifecycle;
pub(crate) mod turn_complete;
mod workspace_tree;

use serde_json::Value;

use crate::domain::workflow::{
    FacetKind, FacetRepository, FacetSummary, ManagedWorktreeGateway, RunListFilter,
    RunStatusFilter, SecretSourceGateway, WorkflowDefinition, WorkflowDefinitionRepository,
    WorkflowError, WorkflowRunArchiveRepository, WorkflowRunSummary, WorkflowStateSnapshot,
    WorkflowSummary,
};
use crate::usecase::workflow::ports::{
    ExternalEditorGateway, WorkflowConfigPathGateway, WorkflowDefinitionSourceGateway,
    WorkflowDiagnosticsGateway,
};

use definition::WorkflowDefinitionUsecase;
use facet::WorkflowFacetUsecase;
use output::WorkflowOutputUsecase;
pub use output::WorkflowValidateOutputResult;
use query_service::WorkflowQueryService;
pub use query_service::{WorkflowEventView, WorkflowGetOutputResult, WorkflowStepDetailView};
pub use runtime_command::WorkflowRuntimeUsecase;
pub(crate) use step_lifecycle::WorkflowStepLifecycleUsecase;
pub(crate) use workspace_tree::{
    WorkspaceSessionGateway, WorkspaceSessionInput, WorkspaceSessionState, WorkspaceTreeNodeDto,
    WorkspaceWorkflowHistoryItemDto, WorkspaceWorkflowStepNodeDto,
};

#[derive(Clone)]
pub struct WorkflowUsecase {
    query: WorkflowQueryService,
    definition_commands: WorkflowDefinitionUsecase,
    facet_commands: WorkflowFacetUsecase,
    output: WorkflowOutputUsecase,
    worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
    editors: std::sync::Arc<dyn ExternalEditorGateway>,
    diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
    config_paths: std::sync::Arc<dyn WorkflowConfigPathGateway>,
    sessions: std::sync::Arc<dyn WorkspaceSessionGateway>,
    archive_runs: std::sync::Arc<dyn WorkflowRunArchiveRepository>,
}

impl WorkflowUsecase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        query: WorkflowQueryService,
        definitions: std::sync::Arc<dyn WorkflowDefinitionRepository>,
        definition_sources: std::sync::Arc<dyn WorkflowDefinitionSourceGateway>,
        facets: std::sync::Arc<dyn FacetRepository>,
        worktrees: std::sync::Arc<dyn ManagedWorktreeGateway>,
        editors: std::sync::Arc<dyn ExternalEditorGateway>,
        diagnostics: std::sync::Arc<dyn WorkflowDiagnosticsGateway>,
        config_paths: std::sync::Arc<dyn WorkflowConfigPathGateway>,
        secrets: std::sync::Arc<dyn SecretSourceGateway>,
        sessions: std::sync::Arc<dyn WorkspaceSessionGateway>,
        archive_runs: std::sync::Arc<dyn WorkflowRunArchiveRepository>,
    ) -> Self {
        let definition_commands = WorkflowDefinitionUsecase::new(definitions, definition_sources);
        let facet_commands = WorkflowFacetUsecase::new(facets.clone());
        let output = WorkflowOutputUsecase::new(query.clone(), secrets);
        Self {
            query,
            definition_commands,
            facet_commands,
            output,
            worktrees,
            editors,
            diagnostics,
            config_paths,
            sessions,
            archive_runs,
        }
    }

    pub fn list_runs(
        &self,
        filter: RunListFilter,
    ) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
        self.query.list_runs(filter)
    }

    pub fn list_runs_for_worktree(
        &self,
        status: Option<RunStatusFilter>,
        worktree_path: &str,
    ) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        self.query.list_runs(RunListFilter {
            status,
            worktree_path: Some(worktree_path),
        })
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
        self.query.get_run(run_id)
    }

    pub fn authorize_run_summary(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
        let Some(summary) = self.get_run(run_id)? else {
            return Ok(None);
        };
        match self.resolve_worktree_path(&summary.worktree_path) {
            Ok(_) => Ok(Some(summary)),
            Err(_) => Ok(None),
        }
    }

    pub fn authorize_run_summary_for_worktree(
        &self,
        run_id: &str,
        worktree_path: &str,
    ) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
        let canonical = self.resolve_worktree_path(worktree_path)?;
        let Some(summary) = self.authorize_run_summary(run_id)? else {
            return Ok(None);
        };
        if summary.worktree_path == canonical {
            Ok(Some(summary))
        } else {
            Ok(None)
        }
    }

    pub fn resolve_worktree_by_run(&self, run_id: &str) -> Result<Option<String>, WorkflowError> {
        self.query.resolve_worktree_by_run(run_id)
    }

    pub fn resolve_worktree_path(&self, worktree_path: &str) -> Result<String, WorkflowError> {
        self.worktrees.resolve(worktree_path)
    }

    pub fn list_workflows(
        &self,
        running_names: &[String],
    ) -> Result<Vec<WorkflowSummary>, WorkflowError> {
        self.query.list_workflows(running_names)
    }

    pub fn get_workflow(
        &self,
        file_stem: &str,
    ) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        self.query.get_workflow(file_stem)
    }

    pub fn get_workflow_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        self.query.get_workflow_source(file_stem)
    }

    pub fn get_run_log(&self, run_id: &str) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        self.query.get_run_log(run_id)
    }

    pub fn get_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
        self.query.get_run_state(run_id)
    }

    pub fn get_step_detail(
        &self,
        run_id: &str,
        node_name: &str,
        run_index: Option<u32>,
    ) -> Result<Option<WorkflowStepDetailView>, WorkflowError> {
        self.query.get_step_detail(run_id, node_name, run_index)
    }

    pub fn list_facets(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        self.query.list_facets(kind)
    }

    pub fn get_facet(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
        self.query.get_facet(kind, key)
    }

    pub fn list_facet_summaries(
        &self,
        kind: FacetKind,
    ) -> Result<Vec<FacetSummary>, WorkflowError> {
        self.query.list_facet_summaries(kind)
    }

    pub fn save_workflow_source(
        &self,
        source: &str,
        original_name: Option<&str>,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        self.definition_commands
            .save_workflow_source(source, original_name)
    }

    pub fn delete_workflow(&self, name: &str) -> Result<(), WorkflowError> {
        self.definition_commands.delete_workflow(name)
    }

    pub fn duplicate_workflow(
        &self,
        source_name: &str,
        new_name: &str,
    ) -> Result<(), WorkflowError> {
        self.definition_commands
            .duplicate_workflow(source_name, new_name)
    }

    pub fn save_facet(
        &self,
        kind: FacetKind,
        key: &str,
        content: &str,
        is_new: bool,
    ) -> Result<(), WorkflowError> {
        self.facet_commands.save_facet(kind, key, content, is_new)
    }

    pub fn delete_facet(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.facet_commands.delete_facet(kind, key)
    }

    pub fn duplicate_facet(
        &self,
        kind: FacetKind,
        source_key: &str,
        new_key: &str,
    ) -> Result<(), WorkflowError> {
        self.facet_commands
            .duplicate_facet(kind, source_key, new_key)
    }

    pub fn open_workflow_in_editor(&self, name: &str) -> Result<(), WorkflowError> {
        self.editors.open_workflow(name)
    }

    pub fn open_facet_in_editor(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
        self.editors.open_facet(kind.dir_name(), key)
    }

    pub fn diagnose_all(&self) -> Result<serde_json::Value, WorkflowError> {
        self.diagnostics.diagnose_all()
    }

    pub fn automation_config_dir(&self) -> Result<String, WorkflowError> {
        self.config_paths.automation_config_dir()
    }

    pub fn render_facet_preview(
        &self,
        content: &str,
        sample_values: &std::collections::HashMap<String, String>,
    ) -> String {
        self.facet_commands
            .render_facet_preview(content, sample_values)
    }

    pub fn validate_output(
        &self,
        run_id: &str,
        step_name: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        self.output
            .validate_output(run_id, step_name, structured_output)
    }

    pub fn get_output(
        &self,
        run_id: &str,
        step_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        self.output.get_output(run_id, step_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        RunId, RunListFilter, RunStatus, RunStatusFilter, TriggerSource,
        WorkflowRunManualArchiveRecord, WorkflowRunRecord, WorkflowRunRepository,
    };
    use crate::usecase::workflow::ports::{
        ExternalEditorGateway, WorkflowConfigPathGateway, WorkflowDiagnosticsGateway,
        WorkflowEventDraft, WorkflowEventRepository, WorkflowStateProjectionRepository,
        WorkflowStepDetailProjectionRepository,
    };
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("read source dir {}: {err}", dir.display()))
        {
            let path = entry
                .unwrap_or_else(|err| panic!("read source entry in {}: {err}", dir.display()))
                .path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }

    fn assert_no_forbidden_crate_dependencies(relative_dir: &str, forbidden_modules: &[&str]) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let layer_root = source_root.join(relative_dir);
        let mut files = Vec::new();
        collect_rs_files(&layer_root, &mut files);
        files.sort();

        assert!(
            !files.is_empty(),
            "no workflow source files found under {relative_dir}"
        );

        let forbidden_refs = forbidden_modules
            .iter()
            .map(|module| format!("crate::{module}"))
            .collect::<Vec<_>>();
        let mut violations = Vec::new();

        for file in files {
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
            for forbidden in &forbidden_refs {
                if content.contains(forbidden) {
                    let display_path = file.strip_prefix(&source_root).unwrap_or(&file);
                    violations.push(format!("{} -> {forbidden}", display_path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "workflow layer dependency violations:\n{}",
            violations.join("\n")
        );
    }

    fn production_source(content: &str) -> String {
        let mut lines = Vec::new();
        for line in content.lines() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    fn production_source_excluding_cfg_test_items(content: &str) -> String {
        let lines = content.lines().collect::<Vec<_>>();
        let mut production = Vec::new();
        let mut index = 0;

        while index < lines.len() {
            let line = lines[index];
            if line.trim_start().starts_with("#[cfg(test)]") {
                index = skip_cfg_test_item(&lines, index + 1);
            } else {
                production.push(line);
                index += 1;
            }
        }

        production.join("\n")
    }

    fn skip_cfg_test_item(lines: &[&str], mut index: usize) -> usize {
        while index < lines.len() && lines[index].trim().is_empty() {
            index += 1;
        }
        while index < lines.len() && lines[index].trim_start().starts_with("#[") {
            index += 1;
            while index < lines.len() && lines[index].trim().is_empty() {
                index += 1;
            }
        }

        let mut depth = 0_i32;
        let mut saw_brace = false;
        while index < lines.len() {
            let line = lines[index];
            for ch in line.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        saw_brace = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }

            let trimmed = line.trim_end();
            index += 1;
            if saw_brace && depth <= 0 {
                break;
            }
            if !saw_brace && (trimmed.ends_with(';') || trimmed.ends_with(',')) {
                break;
            }
        }

        index
    }

    fn assert_no_forbidden_production_patterns(relative_dir: &str, forbidden_patterns: &[&str]) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let layer_root = source_root.join(relative_dir);
        let mut files = Vec::new();
        collect_rs_files(&layer_root, &mut files);
        files.sort();

        assert!(
            !files.is_empty(),
            "no workflow source files found under {relative_dir}"
        );

        let mut violations = Vec::new();
        for file in files {
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
            let production = production_source(&content);
            for forbidden in forbidden_patterns {
                if production.contains(forbidden) {
                    let display_path = file.strip_prefix(&source_root).unwrap_or(&file);
                    violations.push(format!("{} -> {forbidden}", display_path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "workflow production dependency violations:\n{}",
            violations.join("\n")
        );
    }

    fn assert_no_forbidden_production_patterns_in_file(
        relative_file: &str,
        forbidden_patterns: &[&str],
    ) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let file = source_root.join(relative_file);
        let content = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
        let production = production_source_excluding_cfg_test_items(&content);
        let mut violations = Vec::new();

        for forbidden in forbidden_patterns {
            if production.contains(forbidden) {
                violations.push(format!("{relative_file} -> {forbidden}"));
            }
        }

        assert!(
            violations.is_empty(),
            "workflow production dependency violations:\n{}",
            violations.join("\n")
        );
    }

    fn assert_no_forbidden_production_patterns_in_all_sources(
        relative_dir: &str,
        forbidden_patterns: &[&str],
    ) {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let layer_root = source_root.join(relative_dir);
        let mut files = Vec::new();
        collect_rs_files(&layer_root, &mut files);
        files.sort();

        assert!(
            !files.is_empty(),
            "no source files found under {relative_dir}"
        );

        let mut violations = Vec::new();
        for file in files {
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("read source file {}: {err}", file.display()));
            let production = production_source_excluding_cfg_test_items(&content);
            for forbidden in forbidden_patterns {
                if production.contains(forbidden) {
                    let display_path = file.strip_prefix(&source_root).unwrap_or(&file);
                    violations.push(format!("{} -> {forbidden}", display_path.display()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "production dependency violations:\n{}",
            violations.join("\n")
        );
    }

    #[derive(Default)]
    struct NoopRunRepository;

    impl WorkflowRunRepository for NoopRunRepository {
        fn register_active(&self, _run: WorkflowRunRecord) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn complete_run(
            &self,
            _run_id: &RunId,
            _completed: WorkflowRunRecord,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn list_runs(
            &self,
            _filter: RunListFilter,
        ) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
            Ok(Vec::new())
        }

        fn get_run(&self, _run_id: &RunId) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
            Ok(None)
        }

        fn resolve_active_run_by_worktree(
            &self,
            _worktree_path: &str,
        ) -> Result<Option<RunId>, WorkflowError> {
            Ok(None)
        }

        fn resolve_worktree_by_run(
            &self,
            _run_id: &RunId,
        ) -> Result<Option<String>, WorkflowError> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct FakeRunRepository {
        runs: Mutex<HashMap<String, WorkflowRunSummary>>,
    }

    impl FakeRunRepository {
        fn insert(&self, run: WorkflowRunSummary) {
            self.runs.lock().unwrap().insert(run.run_id.clone(), run);
        }
    }

    impl WorkflowRunRepository for FakeRunRepository {
        fn register_active(&self, _run: WorkflowRunRecord) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn complete_run(
            &self,
            _run_id: &RunId,
            _completed: WorkflowRunRecord,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn list_runs(
            &self,
            filter: RunListFilter,
        ) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
            let mut runs = self
                .runs
                .lock()
                .unwrap()
                .values()
                .filter(|run| match filter.status {
                    Some(RunStatusFilter::Active) => !run.status.is_terminal(),
                    Some(RunStatusFilter::Terminal) => run.status.is_terminal(),
                    None => true,
                })
                .filter(|run| {
                    filter
                        .worktree_path
                        .as_ref()
                        .is_none_or(|path| run.worktree_path == *path)
                })
                .cloned()
                .collect::<Vec<_>>();
            runs.sort_by(|a, b| a.run_id.cmp(&b.run_id));
            Ok(runs)
        }

        fn get_run(&self, run_id: &RunId) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
            Ok(self.runs.lock().unwrap().get(run_id.as_str()).cloned())
        }

        fn resolve_active_run_by_worktree(
            &self,
            worktree_path: &str,
        ) -> Result<Option<RunId>, WorkflowError> {
            let run_id = self
                .runs
                .lock()
                .unwrap()
                .values()
                .find(|run| run.worktree_path == worktree_path && !run.status.is_terminal())
                .map(|run| run.run_id.clone());
            run_id.map(RunId::new).transpose()
        }

        fn resolve_worktree_by_run(&self, run_id: &RunId) -> Result<Option<String>, WorkflowError> {
            Ok(self
                .runs
                .lock()
                .unwrap()
                .get(run_id.as_str())
                .map(|run| run.worktree_path.clone()))
        }
    }

    #[derive(Default)]
    struct FakeDefinitionRepository {
        definitions: Mutex<HashMap<String, WorkflowDefinition>>,
    }

    impl WorkflowDefinitionRepository for FakeDefinitionRepository {
        fn list(&self, _running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
            Ok(Vec::new())
        }

        fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
            Ok(self.definitions.lock().unwrap().get(file_stem).cloned())
        }

        fn save(
            &self,
            definition: WorkflowDefinition,
            _original_name: Option<&str>,
        ) -> Result<(), WorkflowError> {
            self.definitions
                .lock()
                .unwrap()
                .insert(definition.name.clone(), definition);
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), WorkflowError> {
            self.definitions.lock().unwrap().remove(name);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeDefinitionSourceGateway {
        sources: Mutex<HashMap<String, String>>,
        save_definition: Mutex<Option<WorkflowDefinition>>,
        save_error: Mutex<Option<String>>,
        saves: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FakeDefinitionSourceGateway {
        fn insert_source(&self, file_stem: &str, source: &str) {
            self.sources
                .lock()
                .unwrap()
                .insert(file_stem.to_string(), source.to_string());
        }

        fn set_save_definition(&self, definition: WorkflowDefinition) {
            *self.save_definition.lock().unwrap() = Some(definition);
        }

        fn fail_saves(&self, message: &str) {
            *self.save_error.lock().unwrap() = Some(message.to_string());
        }

        fn saves(&self) -> Vec<(String, Option<String>)> {
            self.saves.lock().unwrap().clone()
        }
    }

    impl WorkflowDefinitionSourceGateway for FakeDefinitionSourceGateway {
        fn get_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
            Ok(self.sources.lock().unwrap().get(file_stem).cloned())
        }

        fn save_source(
            &self,
            source: &str,
            original_name: Option<&str>,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            self.saves
                .lock()
                .unwrap()
                .push((source.to_string(), original_name.map(str::to_string)));

            if let Some(message) = self.save_error.lock().unwrap().clone() {
                return Err(WorkflowError::external(message));
            }

            Ok(self
                .save_definition
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| workflow_definition("saved-workflow")))
        }
    }

    #[derive(Default)]
    struct FakeFacetRepository {
        facets: Mutex<HashMap<(FacetKind, String), String>>,
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
    struct FakeEventRepository {
        events: Mutex<Vec<WorkflowEventDraft>>,
    }

    impl WorkflowEventRepository for FakeEventRepository {
        fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }

        fn append_batch(&self, events: &[WorkflowEventDraft]) -> Result<(), WorkflowError> {
            self.events.lock().unwrap().extend_from_slice(events);
            Ok(())
        }

        fn read(&self, _run_id: &RunId) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    struct NoopStateProjectionRepository;

    impl WorkflowStateProjectionRepository for NoopStateProjectionRepository {
        fn get_state(
            &self,
            _run_id: &RunId,
        ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
            Ok(None)
        }
    }

    struct NoopStepDetailProjectionRepository;

    impl WorkflowStepDetailProjectionRepository for NoopStepDetailProjectionRepository {
        fn get_step_detail(
            &self,
            _run_id: &RunId,
            _node_name: &str,
            _run_index: Option<u32>,
        ) -> Result<Option<serde_json::Value>, WorkflowError> {
            Ok(None)
        }
    }

    struct NoopArchiveRepository;

    impl WorkflowRunArchiveRepository for NoopArchiveRepository {
        fn archive_manual(&self, _run_id: &RunId, _archived_at: f64) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn restore_manual(&self, _run_id: &RunId, _restored_at: f64) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn manual_archive_records(
            &self,
        ) -> Result<Vec<WorkflowRunManualArchiveRecord>, WorkflowError> {
            Ok(Vec::new())
        }
    }

    struct FakeManagedWorktreeGateway;

    impl ManagedWorktreeGateway for FakeManagedWorktreeGateway {
        fn resolve(&self, worktree_path: &str) -> Result<String, WorkflowError> {
            if worktree_path == "reject" {
                return Err(WorkflowError::external("not managed"));
            }
            Ok(format!("/canonical/{worktree_path}"))
        }
    }

    #[derive(Default)]
    struct FakeExternalEditorGateway {
        opened: Mutex<Vec<String>>,
    }

    impl FakeExternalEditorGateway {
        fn opened(&self) -> Vec<String> {
            self.opened.lock().unwrap().clone()
        }
    }

    impl ExternalEditorGateway for FakeExternalEditorGateway {
        fn open_workflow(&self, name: &str) -> Result<(), WorkflowError> {
            self.opened.lock().unwrap().push(format!("workflow:{name}"));
            Ok(())
        }

        fn open_facet(&self, kind: &str, key: &str) -> Result<(), WorkflowError> {
            self.opened
                .lock()
                .unwrap()
                .push(format!("facet:{kind}:{key}"));
            Ok(())
        }
    }

    struct FakeDiagnosticsGateway;

    impl WorkflowDiagnosticsGateway for FakeDiagnosticsGateway {
        fn diagnose_all(&self) -> Result<serde_json::Value, WorkflowError> {
            Ok(serde_json::json!({"items": [], "workflowSummaries": {}}))
        }
    }

    struct FakeConfigPathGateway;

    impl WorkflowConfigPathGateway for FakeConfigPathGateway {
        fn automation_config_dir(&self) -> Result<String, WorkflowError> {
            Ok("/automation".to_string())
        }
    }

    struct FakeSecretSourceGateway;

    impl SecretSourceGateway for FakeSecretSourceGateway {
        fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
            Ok(vec!["token-123".to_string()])
        }
    }

    struct EmptyWorkspaceSessionGateway;

    impl WorkspaceSessionGateway for EmptyWorkspaceSessionGateway {
        fn list_active_sessions(
            &self,
            _worktree_path: &str,
        ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
            Ok(Vec::new())
        }

        fn list_closed_sessions(
            &self,
            _worktree_path: &str,
        ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
            Ok(Vec::new())
        }
    }

    struct Fixture {
        usecase: WorkflowUsecase,
        editors: Arc<FakeExternalEditorGateway>,
        definition_sources: Arc<FakeDefinitionSourceGateway>,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_runs(Arc::new(NoopRunRepository))
        }

        fn with_runs(runs: Arc<dyn WorkflowRunRepository>) -> Self {
            Self::with_runs_and_definition_sources(
                runs,
                Arc::new(FakeDefinitionSourceGateway::default()),
            )
        }

        fn with_definition_sources(definition_sources: Arc<FakeDefinitionSourceGateway>) -> Self {
            Self::with_runs_and_definition_sources(Arc::new(NoopRunRepository), definition_sources)
        }

        fn with_runs_and_definition_sources(
            runs: Arc<dyn WorkflowRunRepository>,
            definition_sources: Arc<FakeDefinitionSourceGateway>,
        ) -> Self {
            let definitions = Arc::new(FakeDefinitionRepository::default());
            let facets = Arc::new(FakeFacetRepository::default());
            let events = Arc::new(FakeEventRepository::default());
            let editors = Arc::new(FakeExternalEditorGateway::default());
            let query = WorkflowQueryService::new(
                runs,
                definitions.clone(),
                definition_sources.clone(),
                facets.clone(),
                events.clone(),
                Arc::new(NoopStateProjectionRepository),
                Arc::new(NoopStepDetailProjectionRepository),
            );
            let usecase = WorkflowUsecase::new(
                query,
                definitions.clone(),
                definition_sources.clone(),
                facets.clone(),
                Arc::new(FakeManagedWorktreeGateway),
                editors.clone(),
                Arc::new(FakeDiagnosticsGateway),
                Arc::new(FakeConfigPathGateway),
                Arc::new(FakeSecretSourceGateway),
                Arc::new(EmptyWorkspaceSessionGateway),
                Arc::new(NoopArchiveRepository),
            );
            Self {
                usecase,
                editors,
                definition_sources,
            }
        }
    }

    fn workflow_definition(name: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            name: name.to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: Vec::new(),
        }
    }

    fn run_summary(run_id: &str, worktree_path: &str, status: RunStatus) -> WorkflowRunSummary {
        WorkflowRunSummary {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            task: None,
            status,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("node".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
        }
    }

    #[test]
    fn resolve_worktree_path_delegates_to_managed_worktree_gateway() {
        let fixture = Fixture::new();

        assert_eq!(
            fixture.usecase.resolve_worktree_path("repo").unwrap(),
            "/canonical/repo"
        );
        assert!(fixture.usecase.resolve_worktree_path("reject").is_err());
    }

    #[test]
    fn list_runs_for_worktree_canonicalizes_path_before_querying_runs() {
        let runs = Arc::new(FakeRunRepository::default());
        runs.insert(run_summary(
            "00000000-0000-0000-0000-000000000001",
            "/canonical/repo",
            RunStatus::Running,
        ));
        runs.insert(run_summary(
            "00000000-0000-0000-0000-000000000002",
            "/canonical/repo",
            RunStatus::Completed,
        ));
        runs.insert(run_summary(
            "00000000-0000-0000-0000-000000000003",
            "/canonical/other",
            RunStatus::Running,
        ));
        let fixture = Fixture::with_runs(runs);

        let listed = fixture
            .usecase
            .list_runs_for_worktree(Some(RunStatusFilter::Active), "repo")
            .unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].run_id, "00000000-0000-0000-0000-000000000001");
        assert!(fixture
            .usecase
            .list_runs_for_worktree(None, "reject")
            .is_err());
    }

    #[test]
    fn get_workflow_source_returns_some_and_none_from_gateway() {
        let definition_sources = Arc::new(FakeDefinitionSourceGateway::default());
        definition_sources.insert_source("wf", "name: wf\n");
        let fixture = Fixture::with_definition_sources(definition_sources);

        assert_eq!(
            fixture.usecase.get_workflow_source("wf").unwrap(),
            Some("name: wf\n".to_string())
        );
        assert_eq!(
            fixture.usecase.get_workflow_source("missing").unwrap(),
            None
        );
    }

    #[test]
    fn save_workflow_source_returns_saved_definition_and_surfaces_gateway_errors() {
        let fixture = Fixture::new();
        fixture
            .definition_sources
            .set_save_definition(workflow_definition("saved-wf"));

        let saved = fixture
            .usecase
            .save_workflow_source("name: saved-wf\n", Some("old-wf"))
            .unwrap();

        assert_eq!(saved.name, "saved-wf");
        assert_eq!(
            fixture.definition_sources.saves(),
            vec![("name: saved-wf\n".to_string(), Some("old-wf".to_string()))]
        );

        fixture.definition_sources.fail_saves("save failed");

        assert!(fixture
            .usecase
            .save_workflow_source("name: failed-wf\n", None)
            .is_err());
    }

    #[test]
    fn authorize_run_summary_for_worktree_hides_unmanaged_or_mismatched_runs() {
        let runs = Arc::new(FakeRunRepository::default());
        runs.insert(run_summary(
            "00000000-0000-0000-0000-000000000011",
            "/canonical/repo",
            RunStatus::Running,
        ));
        runs.insert(run_summary(
            "00000000-0000-0000-0000-000000000012",
            "reject",
            RunStatus::Running,
        ));
        let fixture = Fixture::with_runs(runs);

        let authorized = fixture
            .usecase
            .authorize_run_summary_for_worktree("00000000-0000-0000-0000-000000000011", "repo")
            .unwrap();
        assert!(authorized.is_some());

        let mismatched = fixture
            .usecase
            .authorize_run_summary_for_worktree("00000000-0000-0000-0000-000000000011", "other")
            .unwrap();
        assert!(mismatched.is_none());

        let unmanaged = fixture
            .usecase
            .authorize_run_summary("00000000-0000-0000-0000-000000000012")
            .unwrap();
        assert!(unmanaged.is_none());
    }

    #[test]
    fn editor_commands_delegate_to_external_editor_gateway() {
        let fixture = Fixture::new();

        fixture
            .usecase
            .open_workflow_in_editor("custom-workflow")
            .unwrap();
        fixture
            .usecase
            .open_facet_in_editor(FacetKind::Instruction, "implement")
            .unwrap();

        assert_eq!(
            fixture.editors.opened(),
            vec![
                "workflow:custom-workflow".to_string(),
                "facet:instructions:implement".to_string(),
            ]
        );
    }

    #[test]
    fn diagnose_and_config_path_delegate_to_gateways() {
        let fixture = Fixture::new();

        let report = fixture.usecase.diagnose_all().unwrap();
        assert_eq!(report["items"].as_array().unwrap().len(), 0);
        assert_eq!(
            fixture.usecase.automation_config_dir().unwrap(),
            "/automation"
        );
    }

    #[test]
    fn workflow_domain_and_usecase_keep_dependencies_inward() {
        let outer_modules = [
            "adaptor",
            "agent_message_dispatcher",
            "app_data_dir",
            "cli",
            "cli_install",
            "config",
            "external_editor",
            "focus_tracker",
            "git",
            "git_host",
            "infrastructure",
            "mcp",
            "menu",
            "native_drop",
            "notion",
            "other",
            "path_aliases",
            "permission",
            "protocol",
            "pty",
            "qr_code",
            "review_comments",
            concat!("session", "_commands"),
            "shell_integration",
            "tls",
            "tray",
            "vpn_detect",
            "watcher",
            "webhook",
            "workflow",
            "workspace_state_store",
        ];
        let mut domain_forbidden = outer_modules.to_vec();
        domain_forbidden.push("usecase");

        assert_no_forbidden_crate_dependencies("domain/workflow", &domain_forbidden);
        assert_no_forbidden_crate_dependencies("usecase/workflow", &outer_modules);

        let external_dependency_patterns = [
            "tauri::",
            "git2::",
            "tokio::",
            "reqwest::",
            "sqlx::",
            "std::env::",
            "std::fs::",
            "std::net::",
            "std::process::",
            "use std::fs",
            "canonicalize(",
        ];
        assert_no_forbidden_production_patterns("domain/workflow", &external_dependency_patterns);
        assert_no_forbidden_production_patterns("usecase/workflow", &external_dependency_patterns);
        assert_no_forbidden_production_patterns(
            "usecase/repository_state",
            &external_dependency_patterns,
        );
    }

    #[test]
    fn domain_and_usecase_production_do_not_depend_on_protocol_dto() {
        let forbidden_patterns = [
            concat!("crate", "::", "adaptor", "::", "protocol"),
            concat!("adaptor", "::", "protocol"),
        ];

        assert_no_forbidden_production_patterns_in_all_sources("domain", &forbidden_patterns);
        assert_no_forbidden_production_patterns_in_all_sources("usecase", &forbidden_patterns);
    }

    #[test]
    fn workflow_controller_production_uses_usecase_boundary_only() {
        let forbidden_patterns = [
            concat!("crate", "::", "adaptor", "::gateway", "::workflow"),
            concat!("adaptor", "::gateway", "::workflow"),
            concat!("crate", "::", "infrastructure", "::workflow"),
            concat!("infrastructure", "::workflow"),
        ];

        assert_no_forbidden_production_patterns(
            "adaptor/controller/command/workflow",
            &forbidden_patterns,
        );
        assert_no_forbidden_production_patterns(
            "adaptor/controller/command/agent_session",
            &[concat!(
                "crate",
                "::",
                "adaptor",
                "::",
                "controller",
                "::",
                "handler"
            )],
        );
    }

    #[test]
    fn agent_session_runtime_status_notification_uses_usecase_port() {
        assert_no_forbidden_production_patterns_in_file(
            "usecase/agent_session/runtime/ports.rs",
            &[
                concat!("crate", "::", "adaptor", "::", "presenter"),
                concat!("crate", "::", "adaptor", "::", "gateway"),
            ],
        );
    }

    #[test]
    fn workflow_legacy_entrypoints_are_removed() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for relative_path in [
            "workflow",
            concat!("infrastructure", "/workflow"),
            concat!("workflow_state", "_events.rs"),
            concat!("workflow_state", "_presenter.rs"),
            concat!("workflow_step", "_lifecycle.rs"),
            concat!("workflow_step", "_lifecycle_adapters.rs"),
            concat!("session", "_commands.rs"),
            concat!("protocol", "/workflow.rs"),
        ] {
            assert!(
                !source_root.join(relative_path).exists(),
                "{relative_path} must not remain as a workflow compatibility shim"
            );
        }
    }

    #[test]
    fn transport_legacy_entrypoints_are_removed() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for relative_path in [
            "ws_server",
            concat!("ws_server", ".rs"),
            concat!("ws_bridge", ".rs"),
            "adaptor/controller/handler",
            concat!("agent_status", "_events.rs"),
        ] {
            assert!(
                !source_root.join(relative_path).exists(),
                "{relative_path} must not remain as a transport compatibility path"
            );
        }
    }
}
