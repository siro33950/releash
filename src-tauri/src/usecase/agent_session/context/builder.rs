use std::collections::HashMap;
use std::path::PathBuf;

use crate::domain::agent_session::{
    latest_revisions_by_kind, next_epoch_for_identity, replacement_action, snapshot_is_stale,
    ContextEpoch, ContextEpochId, ContextEpochIdentity, ContextRevision, ContextSnapshot,
    ContextSourceKind, ContextSourceState, ReplacementAction, ReplacementTrigger,
};
use crate::domain::code::MentionReference;
use crate::usecase::agent_session::context_meta::{
    context_source_kind_key, ContextEpochMeta, ContextSourceRevisionMeta,
};

use super::instruction_resolver::{
    InstructionResolutionRequest, InstructionResolver, InstructionSourcePort,
};
use super::ports::{BranchDiffContextPort, BranchDiffContextSummary};

pub(crate) fn stable_content_fingerprint(content: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}").expect("writing sha256 digest hex cannot fail");
    }
    hex
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SystemContextInput {
    pub repo_summary: Option<String>,
    pub diff_review_snapshot: Option<String>,
    pub open_editor_selection: Option<String>,
    pub mentions: Option<String>,
    pub terminal_log_summary: Option<String>,
    pub workflow_state: Option<String>,
    pub project_instructions: Option<String>,
    pub backend_model_identity: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SystemContextEditorInput {
    pub active_editor_path: Option<String>,
    pub open_editor_paths: Vec<String>,
    pub selection_file_path: Option<String>,
    pub payload: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SystemContextBuildRequest<'a> {
    pub worktree_path: &'a str,
    pub previous_meta: Option<&'a ContextEpochMeta>,
    pub backend_id: &'a str,
    pub model_id: Option<&'a str>,
    pub mentions: &'a [MentionReference],
    pub editor_context: Option<SystemContextEditorInput>,
    pub read_file_paths: Vec<PathBuf>,
    pub workflow_state: Option<String>,
    pub workflow_instructions: Vec<String>,
}

impl SystemContextInput {
    fn payload_for(&self, kind: ContextSourceKind) -> Option<&str> {
        match kind {
            ContextSourceKind::RepoSummary => self.repo_summary.as_deref(),
            ContextSourceKind::DiffReviewSnapshot => self.diff_review_snapshot.as_deref(),
            ContextSourceKind::OpenEditorSelection => self.open_editor_selection.as_deref(),
            ContextSourceKind::Mentions => self.mentions.as_deref(),
            ContextSourceKind::TerminalLogSummary => self.terminal_log_summary.as_deref(),
            ContextSourceKind::WorkflowState => self.workflow_state.as_deref(),
            ContextSourceKind::ProjectInstructions => self.project_instructions.as_deref(),
            ContextSourceKind::BackendModelIdentity => self.backend_model_identity.as_deref(),
        }
        .filter(|payload| !payload.trim().is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextEpochState {
    pub current_epoch: ContextEpoch,
    pub sources: HashMap<ContextSourceKind, ContextSourceState>,
}

impl ContextEpochState {
    pub fn from_meta(meta: Option<&ContextEpochMeta>, identity: ContextEpochIdentity) -> Self {
        let current = meta.map(ContextEpochMeta::epoch);
        let (current_epoch, _trigger) = next_epoch_for_identity(current.as_ref(), identity);
        let mut sources = HashMap::new();
        for kind in ContextSourceKind::ALL {
            let revision_counter = meta
                .and_then(|meta| meta.revision_for(kind))
                .unwrap_or_default();
            let latest = meta.and_then(|meta| {
                let payload = meta.payload_for(kind)?;
                let fingerprint = meta
                    .fingerprint_for(kind)
                    .map(str::to_string)
                    .unwrap_or_else(|| stable_content_fingerprint(payload));
                Some(ContextSnapshot {
                    kind,
                    epoch_id: current_epoch.id,
                    revision: revision_counter,
                    fingerprint,
                    payload: payload.to_string(),
                })
            });
            sources.insert(
                kind,
                ContextSourceState {
                    kind,
                    latest,
                    revision_counter,
                },
            );
        }
        Self {
            current_epoch,
            sources,
        }
    }

    pub fn to_meta(&self) -> ContextEpochMeta {
        let mut source_revisions = self
            .sources
            .values()
            .map(|state| ContextSourceRevisionMeta {
                kind: context_source_kind_key(state.kind).to_string(),
                revision: state.revision_counter.0,
                fingerprint: state
                    .latest
                    .as_ref()
                    .map(|snapshot| snapshot.fingerprint.clone()),
                payload: state
                    .latest
                    .as_ref()
                    .map(|snapshot| snapshot.payload.clone()),
            })
            .collect::<Vec<_>>();
        source_revisions.sort_by(|a, b| a.kind.cmp(&b.kind));
        ContextEpochMeta {
            epoch_id: self.current_epoch.id.0,
            backend_id: self.current_epoch.backend_id.clone(),
            model_id: self.current_epoch.model_id.clone(),
            worktree_path: self.current_epoch.worktree_path.clone(),
            source_revisions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuiltSystemContext {
    pub state: ContextEpochState,
    pub snapshots: Vec<ContextSnapshot>,
}

impl BuiltSystemContext {
    #[cfg(test)]
    pub fn payload_for(&self, kind: ContextSourceKind) -> Option<&str> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.kind == kind)
            .map(|snapshot| snapshot.payload.as_str())
    }
}

pub(crate) struct SystemContextBuilder;

impl SystemContextBuilder {
    pub fn build(
        previous_meta: Option<&ContextEpochMeta>,
        identity: ContextEpochIdentity,
        trigger_override: Option<ReplacementTrigger>,
        input: SystemContextInput,
    ) -> BuiltSystemContext {
        debug_assert!(ContextSourceKind::MINIMUM_SYSTEM_CONTEXT_SOURCES
            .iter()
            .all(|kind| {
                let definition = kind.definition();
                !definition.meaning.is_empty()
                    && !definition.source.is_empty()
                    && !definition.refresh_trigger.is_empty()
            }));
        let previous_epoch = previous_meta.map(ContextEpochMeta::epoch);
        let (current_epoch, identity_trigger) =
            next_epoch_for_identity(previous_epoch.as_ref(), identity);
        let trigger = merged_replacement_trigger(identity_trigger, trigger_override);
        let mut state = ContextEpochState::from_meta(
            previous_meta,
            ContextEpochIdentity {
                backend_id: current_epoch.backend_id.clone(),
                model_id: current_epoch.model_id.clone(),
                worktree_path: current_epoch.worktree_path.clone(),
            },
        );
        state.current_epoch = current_epoch;

        for kind in ContextSourceKind::ALL {
            let action = replacement_action(trigger, kind);
            let source_state = state
                .sources
                .entry(kind)
                .or_insert_with(|| ContextSourceState::new(kind));
            let clear_on_absent = matches!(
                action,
                ReplacementAction::Discard | ReplacementAction::Rebuild
            );
            match input.payload_for(kind) {
                Some(payload) => {
                    source_state.latest = Some(build_snapshot(
                        kind,
                        state.current_epoch.id,
                        &mut source_state.revision_counter,
                        previous_meta.and_then(|meta| meta.fingerprint_for(kind)),
                        payload,
                    ));
                }
                None if clear_on_absent => {
                    source_state.latest = None;
                }
                None => {}
            }
        }

        let snapshots = state
            .sources
            .values()
            .filter_map(|state| state.latest.clone())
            .collect::<Vec<_>>();
        let latest_revisions = latest_revisions_by_kind(&snapshots);
        let mut snapshots = snapshots
            .into_iter()
            .filter(|snapshot| {
                !snapshot_is_stale(
                    snapshot,
                    state.current_epoch.id,
                    latest_revisions.get(&snapshot.kind).copied(),
                )
            })
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|snapshot| snapshot.kind as u8);

        BuiltSystemContext { state, snapshots }
    }
}

fn build_snapshot(
    kind: ContextSourceKind,
    epoch_id: ContextEpochId,
    revision_counter: &mut ContextRevision,
    previous_fingerprint: Option<&str>,
    payload: &str,
) -> ContextSnapshot {
    let fingerprint = stable_content_fingerprint(payload);
    if previous_fingerprint != Some(fingerprint.as_str()) {
        *revision_counter = revision_counter.next();
    }
    ContextSnapshot {
        kind,
        epoch_id,
        revision: *revision_counter,
        fingerprint,
        payload: payload.to_string(),
    }
}

fn merged_replacement_trigger(
    identity_trigger: ReplacementTrigger,
    trigger_override: Option<ReplacementTrigger>,
) -> ReplacementTrigger {
    if identity_trigger != ReplacementTrigger::None {
        identity_trigger
    } else {
        trigger_override.unwrap_or(ReplacementTrigger::None)
    }
}

pub(crate) fn build_system_context(
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    instruction_source: &dyn InstructionSourcePort,
    request: SystemContextBuildRequest<'_>,
) -> BuiltSystemContext {
    let resolver = InstructionResolver::new(instruction_source);
    let mut read_file_paths = request.read_file_paths;
    read_file_paths.extend(editor_context_read_paths(request.editor_context.as_ref()));
    read_file_paths.extend(mention_read_paths(request.mentions));
    let instructions = resolver.resolve(&InstructionResolutionRequest {
        worktree_root: PathBuf::from(request.worktree_path),
        repo_context_dir: None,
        read_file_paths,
        workflow_instructions: request.workflow_instructions,
    });
    if instructions.skipped_read_errors > 0 {
        log::warn!(
            "Skipped {} project instruction file(s) while building system context",
            instructions.skipped_read_errors
        );
    }
    let project_instructions = instructions.payload();
    let diff_review_snapshot =
        branch_diff_review_snapshot_payload(branch_diff_context, request.worktree_path);
    let previous_instruction_fingerprint = request
        .previous_meta
        .and_then(|meta| meta.fingerprint_for(ContextSourceKind::ProjectInstructions));
    let current_instruction_fingerprint = project_instructions
        .as_deref()
        .map(stable_content_fingerprint);
    let trigger_override = (previous_instruction_fingerprint.is_some()
        && previous_instruction_fingerprint != current_instruction_fingerprint.as_deref())
    .then_some(ReplacementTrigger::InstructionFileChanged);
    SystemContextBuilder::build(
        request.previous_meta,
        ContextEpochIdentity {
            backend_id: Some(request.backend_id.to_string()),
            model_id: request.model_id.map(str::to_string),
            worktree_path: request.worktree_path.to_string(),
        },
        trigger_override,
        SystemContextInput {
            repo_summary: None,
            diff_review_snapshot,
            open_editor_selection: request
                .editor_context
                .as_ref()
                .and_then(|context| context.payload.clone()),
            mentions: mention_payload(request.mentions),
            terminal_log_summary: None,
            workflow_state: request.workflow_state,
            project_instructions,
            backend_model_identity: Some(backend_model_identity_payload(
                request.backend_id,
                request.model_id,
            )),
        },
    )
}

fn editor_context_read_paths(editor_context: Option<&SystemContextEditorInput>) -> Vec<PathBuf> {
    let Some(context) = editor_context else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if let Some(path) = context.active_editor_path.as_ref() {
        paths.push(PathBuf::from(path));
    }
    paths.extend(context.open_editor_paths.iter().map(PathBuf::from));
    if let Some(path) = context.selection_file_path.as_ref() {
        paths.push(PathBuf::from(path));
    }
    paths
}

fn mention_read_paths(mentions: &[MentionReference]) -> Vec<PathBuf> {
    mentions
        .iter()
        .map(|mention| PathBuf::from(&mention.file_path))
        .collect()
}

fn mention_payload(mentions: &[MentionReference]) -> Option<String> {
    if mentions.is_empty() {
        return None;
    }
    let payload = mentions
        .iter()
        .map(|mention| {
            let mut value = mention.file_path.clone();
            if let Some(start_line) = mention.start_line {
                value.push_str(&format!(":{start_line}"));
                if let Some(end_line) = mention.end_line {
                    value.push_str(&format!("-{end_line}"));
                }
            }
            value
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(payload)
}

fn branch_diff_review_snapshot_payload(
    branch_diff_context: Option<&dyn BranchDiffContextPort>,
    worktree_path: &str,
) -> Option<String> {
    let branch_diff_context = branch_diff_context?;
    match branch_diff_context.get_branch_diff_context(worktree_path) {
        Ok(summary) => Some(format_diff_review_snapshot(&summary)),
        Err(err) => {
            log::warn!("Failed to build branch diff system context: {err}");
            None
        }
    }
}

fn format_diff_review_snapshot(summary: &BranchDiffContextSummary) -> String {
    if summary.changed_files.is_empty() {
        return "No changed files.".to_string();
    }
    let mut lines = vec![format!("base_branch: {}", summary.base_branch)];
    for file in summary.changed_files.iter().take(100) {
        lines.push(format!(
            "- {} {} (+{} -{})",
            file.status, file.path, file.stats.additions, file.stats.deletions
        ));
    }
    if summary.changed_files.len() > 100 {
        lines.push(format!(
            "... {} more files",
            summary.changed_files.len().saturating_sub(100)
        ));
    }
    lines.join("\n")
}

fn backend_model_identity_payload(backend_id: &str, model_id: Option<&str>) -> String {
    format!(
        "backend_id: {backend_id}\nmodel_id: {}",
        model_id.unwrap_or("default")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::{ContextEpochId, ContextRevision};
    use crate::usecase::agent_session::context::{
        BranchDiffContextChangedFile, BranchDiffContextStats,
    };
    use crate::usecase::agent_session::session::WorkflowStepContextDto;

    struct EmptyInstructionSource;

    impl InstructionSourcePort for EmptyInstructionSource {
        fn read_instruction_file(
            &self,
            _path: &std::path::Path,
            _worktree_root: &std::path::Path,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    struct FakeBranchDiffContext;

    impl BranchDiffContextPort for FakeBranchDiffContext {
        fn get_branch_diff_context(
            &self,
            _worktree_path: &str,
        ) -> Result<BranchDiffContextSummary, String> {
            Ok(BranchDiffContextSummary {
                base_branch: "main".to_string(),
                changed_files: vec![BranchDiffContextChangedFile {
                    path: "src/lib.rs".to_string(),
                    status: "modified".to_string(),
                    stats: BranchDiffContextStats {
                        additions: 3,
                        deletions: 1,
                    },
                }],
            })
        }
    }

    fn identity(worktree_path: &str) -> ContextEpochIdentity {
        ContextEpochIdentity {
            backend_id: Some("claude".to_string()),
            model_id: Some("sonnet".to_string()),
            worktree_path: worktree_path.to_string(),
        }
    }

    #[test]
    fn context_epoch_meta_serializes_as_usecase_dto() {
        let meta = ContextEpochMeta {
            epoch_id: 3,
            backend_id: Some("claude".to_string()),
            model_id: Some("sonnet".to_string()),
            worktree_path: "/repo".to_string(),
            source_revisions: vec![ContextSourceRevisionMeta {
                kind: "project_instructions".to_string(),
                revision: 7,
                fingerprint: Some("abc".to_string()),
                payload: Some("private payload".to_string()),
            }],
        };

        let json = serde_json::to_value(&meta).expect("serialize");

        assert_eq!(json["epochId"], 3);
        assert_eq!(json["sourceRevisions"][0]["kind"], "project_instructions");
        assert!(json["sourceRevisions"][0].get("payload").is_none());
        assert_eq!(
            meta.epoch(),
            ContextEpoch {
                id: ContextEpochId(3),
                backend_id: Some("claude".to_string()),
                model_id: Some("sonnet".to_string()),
                worktree_path: "/repo".to_string(),
            }
        );
        assert_eq!(
            meta.revision_for(ContextSourceKind::ProjectInstructions),
            Some(ContextRevision(7))
        );
    }

    #[test]
    fn build_holds_sources_with_epoch_and_revision() {
        let built = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                repo_summary: Some("repo".to_string()),
                diff_review_snapshot: Some("diff".to_string()),
                open_editor_selection: Some("editor".to_string()),
                mentions: Some("mentions".to_string()),
                terminal_log_summary: Some("terminal".to_string()),
                workflow_state: Some("workflow".to_string()),
                project_instructions: Some("instructions".to_string()),
                backend_model_identity: Some("system".to_string()),
            },
        );

        assert_eq!(built.state.current_epoch.id.0, 1);
        for kind in ContextSourceKind::MINIMUM_SYSTEM_CONTEXT_SOURCES {
            let snapshot = built
                .snapshots
                .iter()
                .find(|snapshot| snapshot.kind == kind)
                .expect("source snapshot");
            assert_eq!(snapshot.epoch_id, built.state.current_epoch.id);
            assert_eq!(snapshot.revision, ContextRevision(2));
        }
    }

    #[test]
    fn missing_source_does_not_break_other_epoch_revisions() {
        let built = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                repo_summary: Some("repo".to_string()),
                terminal_log_summary: None,
                project_instructions: Some("instructions".to_string()),
                ..SystemContextInput::default()
            },
        );

        assert!(built
            .payload_for(ContextSourceKind::TerminalLogSummary)
            .is_none());
        assert_eq!(
            built.payload_for(ContextSourceKind::RepoSummary),
            Some("repo")
        );
        assert_eq!(
            built.payload_for(ContextSourceKind::ProjectInstructions),
            Some("instructions")
        );
    }

    #[test]
    fn retain_retags_previous_payload_when_fresh_input_is_absent() {
        let first = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                repo_summary: Some("repo-v1".to_string()),
                workflow_state: Some("workflow-v1".to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();

        let next = SystemContextBuilder::build(
            Some(&meta),
            identity("/repo"),
            None,
            SystemContextInput::default(),
        );

        let repo = next
            .snapshots
            .iter()
            .find(|snapshot| snapshot.kind == ContextSourceKind::RepoSummary)
            .expect("retained repo snapshot");
        assert_eq!(repo.payload, "repo-v1");
        assert_eq!(repo.epoch_id, next.state.current_epoch.id);
        assert_eq!(repo.revision, ContextRevision(2));
        assert_eq!(
            next.payload_for(ContextSourceKind::WorkflowState),
            Some("workflow-v1")
        );
    }

    #[test]
    fn backend_change_retags_retained_context_and_rebuilds_backend_specific_sources() {
        let first = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                repo_summary: Some("repo-v1".to_string()),
                project_instructions: Some("claude instructions".to_string()),
                backend_model_identity: Some("claude system".to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();

        let next = SystemContextBuilder::build(
            Some(&meta),
            ContextEpochIdentity {
                backend_id: Some("codex".to_string()),
                model_id: Some("gpt-5".to_string()),
                worktree_path: "/repo".to_string(),
            },
            None,
            SystemContextInput {
                project_instructions: Some("codex instructions".to_string()),
                backend_model_identity: Some("codex system".to_string()),
                ..SystemContextInput::default()
            },
        );

        assert_eq!(
            next.state.current_epoch.id,
            first.state.current_epoch.id.next()
        );
        let repo = next
            .snapshots
            .iter()
            .find(|snapshot| snapshot.kind == ContextSourceKind::RepoSummary)
            .expect("retained repo snapshot");
        assert_eq!(repo.payload, "repo-v1");
        assert_eq!(repo.epoch_id, next.state.current_epoch.id);
        assert_eq!(
            next.payload_for(ContextSourceKind::ProjectInstructions),
            Some("codex instructions")
        );
        assert_eq!(
            next.payload_for(ContextSourceKind::BackendModelIdentity),
            Some("codex system")
        );
        assert!(!next
            .snapshots
            .iter()
            .any(|snapshot| snapshot.payload.contains("claude")));
    }

    #[test]
    fn worktree_change_rebuilds_repo_context_and_advances_epoch() {
        let first = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                repo_summary: Some("repo-v1".to_string()),
                project_instructions: Some("root-v1".to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();

        let next = SystemContextBuilder::build(
            Some(&meta),
            identity("/other"),
            None,
            SystemContextInput {
                repo_summary: Some("repo-v2".to_string()),
                project_instructions: Some("root-v2".to_string()),
                ..SystemContextInput::default()
            },
        );

        assert_eq!(
            next.state.current_epoch.id,
            first.state.current_epoch.id.next()
        );
        assert_eq!(
            next.payload_for(ContextSourceKind::RepoSummary),
            Some("repo-v2")
        );
        assert_eq!(
            next.payload_for(ContextSourceKind::ProjectInstructions),
            Some("root-v2")
        );
    }

    #[test]
    fn backend_change_discards_old_instructions_before_rebuild() {
        let first = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                project_instructions: Some("claude instructions".to_string()),
                backend_model_identity: Some("claude system".to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();

        let next = SystemContextBuilder::build(
            Some(&meta),
            ContextEpochIdentity {
                backend_id: Some("codex".to_string()),
                model_id: Some("gpt-5".to_string()),
                worktree_path: "/repo".to_string(),
            },
            None,
            SystemContextInput {
                project_instructions: Some("codex instructions".to_string()),
                backend_model_identity: Some("codex system".to_string()),
                ..SystemContextInput::default()
            },
        );

        assert_eq!(
            next.payload_for(ContextSourceKind::ProjectInstructions),
            Some("codex instructions")
        );
        assert_eq!(
            next.payload_for(ContextSourceKind::BackendModelIdentity),
            Some("codex system")
        );
        assert!(!next
            .snapshots
            .iter()
            .any(|snapshot| snapshot.payload.contains("claude")));
    }

    #[test]
    fn model_change_discards_old_instructions_before_rebuild() {
        let first = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                project_instructions: Some("sonnet instructions".to_string()),
                backend_model_identity: Some("backend_id: claude\nmodel_id: sonnet".to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();

        let next = SystemContextBuilder::build(
            Some(&meta),
            ContextEpochIdentity {
                backend_id: Some("claude".to_string()),
                model_id: Some("haiku".to_string()),
                worktree_path: "/repo".to_string(),
            },
            None,
            SystemContextInput {
                project_instructions: Some("haiku instructions".to_string()),
                backend_model_identity: Some("backend_id: claude\nmodel_id: haiku".to_string()),
                ..SystemContextInput::default()
            },
        );

        assert_eq!(
            next.payload_for(ContextSourceKind::ProjectInstructions),
            Some("haiku instructions")
        );
        assert_eq!(
            next.payload_for(ContextSourceKind::BackendModelIdentity),
            Some("backend_id: claude\nmodel_id: haiku")
        );
        assert!(!next
            .snapshots
            .iter()
            .any(|snapshot| snapshot.payload.contains("sonnet")));
    }

    #[test]
    fn backend_model_identity_keeps_legacy_key_and_revision_for_same_identity() {
        let payload = "backend_id: claude\nmodel_id: sonnet";
        let first = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                backend_model_identity: Some(payload.to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();
        let expected_fingerprint = stable_content_fingerprint(payload);

        assert!(meta.source_revisions.iter().any(|source| {
            source.kind == "backend_system_prompt"
                && source.fingerprint.as_deref() == Some(expected_fingerprint.as_str())
        }));

        let next = SystemContextBuilder::build(
            Some(&meta),
            identity("/repo"),
            None,
            SystemContextInput {
                backend_model_identity: Some(payload.to_string()),
                ..SystemContextInput::default()
            },
        );

        let first_snapshot = first
            .snapshots
            .iter()
            .find(|snapshot| snapshot.kind == ContextSourceKind::BackendModelIdentity)
            .expect("first identity snapshot");
        let next_snapshot = next
            .snapshots
            .iter()
            .find(|snapshot| snapshot.kind == ContextSourceKind::BackendModelIdentity)
            .expect("next identity snapshot");
        assert_eq!(next_snapshot.revision, first_snapshot.revision);
        assert_eq!(next_snapshot.fingerprint, first_snapshot.fingerprint);
    }

    #[test]
    fn instruction_file_change_rebuilds_only_instruction_source_without_new_epoch() {
        let first = SystemContextBuilder::build(
            None,
            identity("/repo"),
            None,
            SystemContextInput {
                repo_summary: Some("repo".to_string()),
                project_instructions: Some("old".to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();

        let next = SystemContextBuilder::build(
            Some(&meta),
            identity("/repo"),
            Some(ReplacementTrigger::InstructionFileChanged),
            SystemContextInput {
                repo_summary: Some("repo".to_string()),
                project_instructions: Some("new".to_string()),
                ..SystemContextInput::default()
            },
        );

        assert_eq!(next.state.current_epoch.id, first.state.current_epoch.id);
        assert_eq!(
            next.payload_for(ContextSourceKind::RepoSummary),
            Some("repo")
        );
        assert_eq!(
            next.payload_for(ContextSourceKind::ProjectInstructions),
            Some("new")
        );
    }

    #[test]
    fn worktree_change_takes_precedence_over_instruction_file_change_override() {
        let first = SystemContextBuilder::build(
            None,
            identity("/repo-w1"),
            None,
            SystemContextInput {
                repo_summary: Some("repo from w1".to_string()),
                diff_review_snapshot: Some("diff from w1".to_string()),
                open_editor_selection: Some("editor from w1".to_string()),
                mentions: Some("mentions from w1".to_string()),
                project_instructions: Some("instructions from w1".to_string()),
                ..SystemContextInput::default()
            },
        );
        let meta = first.state.to_meta();

        let next = SystemContextBuilder::build(
            Some(&meta),
            identity("/repo-w2"),
            Some(ReplacementTrigger::InstructionFileChanged),
            SystemContextInput {
                project_instructions: Some("instructions from w2".to_string()),
                ..SystemContextInput::default()
            },
        );

        assert_eq!(
            next.state.current_epoch.id,
            first.state.current_epoch.id.next()
        );
        assert!(next.payload_for(ContextSourceKind::RepoSummary).is_none());
        assert!(next
            .payload_for(ContextSourceKind::DiffReviewSnapshot)
            .is_none());
        assert!(next
            .payload_for(ContextSourceKind::OpenEditorSelection)
            .is_none());
        assert!(next.payload_for(ContextSourceKind::Mentions).is_none());
        assert_eq!(
            next.payload_for(ContextSourceKind::ProjectInstructions),
            Some("instructions from w2")
        );
        assert!(!next
            .snapshots
            .iter()
            .any(|snapshot| snapshot.payload.contains("w1")));
    }

    #[test]
    fn build_system_context_uses_branch_diff_only_for_diff_review_snapshot() {
        let built = build_system_context(
            Some(&FakeBranchDiffContext),
            &EmptyInstructionSource,
            SystemContextBuildRequest {
                worktree_path: "/repo",
                previous_meta: None,
                backend_id: "claude",
                model_id: None,
                mentions: &[],
                editor_context: None,
                read_file_paths: Vec::new(),
                workflow_state: None,
                workflow_instructions: Vec::new(),
            },
        );

        assert!(built.payload_for(ContextSourceKind::RepoSummary).is_none());
        assert!(built
            .payload_for(ContextSourceKind::DiffReviewSnapshot)
            .is_some_and(|payload| payload.contains("- modified src/lib.rs (+3 -1)")));
    }

    #[test]
    fn build_system_context_replaces_stale_diff_review_snapshot_from_previous_meta() {
        let old_payload = "old diff payload";
        let old_fingerprint = stable_content_fingerprint(old_payload);
        let previous_meta = ContextEpochMeta {
            epoch_id: 1,
            backend_id: Some("claude".to_string()),
            model_id: None,
            worktree_path: "/repo".to_string(),
            source_revisions: vec![ContextSourceRevisionMeta {
                kind: context_source_kind_key(ContextSourceKind::DiffReviewSnapshot).to_string(),
                revision: 4,
                fingerprint: Some(old_fingerprint),
                payload: Some(old_payload.to_string()),
            }],
        };

        let built = build_system_context(
            Some(&FakeBranchDiffContext),
            &EmptyInstructionSource,
            SystemContextBuildRequest {
                worktree_path: "/repo",
                previous_meta: Some(&previous_meta),
                backend_id: "claude",
                model_id: None,
                mentions: &[],
                editor_context: None,
                read_file_paths: Vec::new(),
                workflow_state: None,
                workflow_instructions: Vec::new(),
            },
        );

        let snapshot = built
            .snapshots
            .iter()
            .find(|snapshot| snapshot.kind == ContextSourceKind::DiffReviewSnapshot)
            .expect("diff review snapshot");
        assert!(snapshot.payload.contains("- modified src/lib.rs (+3 -1)"));
        assert!(!built
            .snapshots
            .iter()
            .any(|snapshot| snapshot.payload.contains(old_payload)));
        assert_eq!(
            snapshot.fingerprint,
            stable_content_fingerprint(&snapshot.payload)
        );
        assert_eq!(snapshot.revision, ContextRevision(5));
    }

    #[test]
    fn build_system_context_does_not_synthesize_terminal_summary_from_transcript() {
        let built = build_system_context(
            None,
            &EmptyInstructionSource,
            SystemContextBuildRequest {
                worktree_path: "/repo",
                previous_meta: None,
                backend_id: "claude",
                model_id: None,
                mentions: &[],
                editor_context: None,
                read_file_paths: Vec::new(),
                workflow_state: None,
                workflow_instructions: Vec::new(),
            },
        );

        assert!(built
            .payload_for(ContextSourceKind::TerminalLogSummary)
            .is_none());
    }

    #[test]
    fn build_system_context_routes_workflow_instruction_outside_workflow_state() {
        let workflow_state = serde_json::to_string(&WorkflowStepContextDto {
            run_id: "run-1".to_string(),
            workflow_name: "wf".to_string(),
            step_name: "step-a".to_string(),
            run_index: 0,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        })
        .expect("workflow state dto serializes");

        let built = build_system_context(
            None,
            &EmptyInstructionSource,
            SystemContextBuildRequest {
                worktree_path: "/repo",
                previous_meta: None,
                backend_id: "claude",
                model_id: None,
                mentions: &[],
                editor_context: None,
                read_file_paths: Vec::new(),
                workflow_state: Some(workflow_state),
                workflow_instructions: vec!["private workflow instruction".to_string()],
            },
        );

        assert!(built
            .payload_for(ContextSourceKind::ProjectInstructions)
            .is_some_and(|payload| payload.contains("private workflow instruction")));
        assert!(built
            .payload_for(ContextSourceKind::WorkflowState)
            .is_some_and(|payload| !payload.contains("private workflow instruction")
                && !payload.contains("parentStepName")
                && !payload.contains("parentRunIndex")));
    }

    #[test]
    fn build_system_context_restores_stored_workflow_instruction() {
        let built = build_system_context(
            None,
            &EmptyInstructionSource,
            SystemContextBuildRequest {
                worktree_path: "/repo",
                previous_meta: None,
                backend_id: "claude",
                model_id: None,
                mentions: &[],
                editor_context: None,
                read_file_paths: Vec::new(),
                workflow_state: None,
                workflow_instructions: vec!["stored workflow instruction".to_string()],
            },
        );

        assert!(built
            .payload_for(ContextSourceKind::ProjectInstructions)
            .is_some_and(|payload| payload.contains("stored workflow instruction")));
    }
}
