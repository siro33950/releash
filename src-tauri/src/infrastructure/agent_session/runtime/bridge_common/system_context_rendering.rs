use crate::domain::agent_session::{ContextSnapshot, ContextSourceKind};
use crate::usecase::agent_session::context::BuiltSystemContext;

pub(super) fn compose_system_prompt(
    system_prompt: Option<String>,
    context: &BuiltSystemContext,
) -> Option<String> {
    let context_blocks = context
        .snapshots
        .iter()
        .filter_map(system_context_block)
        .collect::<Vec<_>>();
    let context_prompt = (!context_blocks.is_empty()).then(|| context_blocks.join("\n\n"));
    let system_prompt = system_prompt.filter(|prompt| !prompt.trim().is_empty());

    match (system_prompt, context_prompt) {
        (Some(prompt), Some(context_prompt)) => Some(format!("{prompt}\n\n{context_prompt}")),
        (None, Some(context_prompt)) => Some(context_prompt),
        (Some(prompt), _) => Some(prompt),
        (None, None) => None,
    }
}

fn system_context_block(snapshot: &ContextSnapshot) -> Option<String> {
    let payload = snapshot.payload.trim();
    if payload.is_empty() {
        return None;
    }
    let tag = match snapshot.kind {
        ContextSourceKind::RepoSummary => "releash_repo_summary",
        ContextSourceKind::DiffReviewSnapshot => "releash_diff_review_snapshot",
        ContextSourceKind::OpenEditorSelection => "releash_open_editor_selection",
        ContextSourceKind::Mentions => "releash_mentions",
        ContextSourceKind::TerminalLogSummary => "releash_terminal_log_summary",
        ContextSourceKind::WorkflowState => "releash_workflow_state",
        ContextSourceKind::ProjectInstructions => "releash_project_instructions",
        ContextSourceKind::BackendModelIdentity => "releash_backend_model_identity",
    };
    Some(format!("<{tag}>\n{payload}\n</{tag}>"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::{
        ContextEpoch, ContextEpochId, ContextRevision, ContextSourceState,
    };
    use crate::usecase::agent_session::context::{BuiltSystemContext, ContextEpochState};
    use std::collections::HashMap;

    fn built_context(snapshots: Vec<ContextSnapshot>) -> BuiltSystemContext {
        BuiltSystemContext {
            state: ContextEpochState {
                current_epoch: ContextEpoch {
                    id: ContextEpochId(1),
                    backend_id: Some("claude".to_string()),
                    model_id: None,
                    worktree_path: "/repo".to_string(),
                },
                sources: HashMap::<ContextSourceKind, ContextSourceState>::new(),
            },
            snapshots,
        }
    }

    fn snapshot(kind: ContextSourceKind, payload: &str) -> ContextSnapshot {
        ContextSnapshot {
            kind,
            epoch_id: ContextEpochId(1),
            revision: ContextRevision(1),
            fingerprint: payload.to_string(),
            payload: payload.to_string(),
        }
    }

    #[test]
    fn compose_system_prompt_adds_project_instruction_block_after_base_prompt() {
        let context = built_context(vec![snapshot(
            ContextSourceKind::ProjectInstructions,
            "Use Rust.",
        )]);

        let prompt = compose_system_prompt(Some("base".to_string()), &context).expect("prompt");

        assert!(prompt.starts_with("base\n\n<releash_project_instructions>"));
        assert!(prompt.contains("Use Rust."));
    }

    #[test]
    fn compose_system_prompt_includes_all_non_empty_source_blocks() {
        let context = built_context(vec![
            snapshot(ContextSourceKind::RepoSummary, "repo"),
            snapshot(ContextSourceKind::DiffReviewSnapshot, "diff"),
            snapshot(ContextSourceKind::TerminalLogSummary, "terminal"),
            snapshot(ContextSourceKind::BackendModelIdentity, "backend"),
        ]);

        let prompt = compose_system_prompt(None, &context).expect("prompt");

        assert!(prompt.contains("<releash_repo_summary>"));
        assert!(prompt.contains("<releash_diff_review_snapshot>"));
        assert!(prompt.contains("<releash_terminal_log_summary>"));
        assert!(prompt.contains("<releash_backend_model_identity>"));
    }

    #[test]
    fn compose_system_prompt_empty_string_treated_as_none_when_context_exists() {
        let context = built_context(vec![snapshot(
            ContextSourceKind::ProjectInstructions,
            "Use Rust.",
        )]);

        let prompt = compose_system_prompt(Some(String::new()), &context).expect("prompt");

        assert_eq!(
            prompt,
            "<releash_project_instructions>\nUse Rust.\n</releash_project_instructions>"
        );
    }

    #[test]
    fn compose_system_prompt_whitespace_string_treated_as_none_when_context_exists() {
        let context = built_context(vec![snapshot(
            ContextSourceKind::ProjectInstructions,
            "Use Rust.",
        )]);

        let prompt = compose_system_prompt(Some(" \n\t ".to_string()), &context).expect("prompt");

        assert_eq!(
            prompt,
            "<releash_project_instructions>\nUse Rust.\n</releash_project_instructions>"
        );
    }

    #[test]
    fn compose_system_prompt_none_without_context_returns_none() {
        let context = built_context(Vec::new());

        let prompt = compose_system_prompt(None, &context);

        assert_eq!(prompt, None);
    }
}
