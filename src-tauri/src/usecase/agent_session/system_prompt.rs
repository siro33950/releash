use std::path::{Path, PathBuf};

use crate::domain::code::MentionReference;
use crate::usecase::agent_session::context::{
    build_system_context, BranchDiffContextPort, BuiltSystemContext, InstructionSourcePort,
    SystemContextBuildRequest, SystemContextEditorInput,
};
use crate::usecase::agent_session::context_meta::ContextEpochMeta;
use crate::usecase::agent_session::session::{
    agent_read_paths_from_messages, ChatSession, SessionStore,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionSystemPromptBuild {
    pub system_context: BuiltSystemContext,
    pub context_epoch: ContextEpochMeta,
    pub workflow_instructions: Vec<String>,
    pub agent_read_paths: Option<Vec<PathBuf>>,
}

pub(crate) struct SessionSystemPromptBuildRequest<'a> {
    pub session_store: &'a SessionStore,
    pub data_dir: &'a Path,
    pub session: &'a ChatSession,
    pub branch_diff_context: Option<&'a dyn BranchDiffContextPort>,
    pub instruction_source: &'a dyn InstructionSourcePort,
    pub backend_id: &'a str,
    pub model_id: Option<&'a str>,
    pub mentions: &'a [MentionReference],
    pub editor_context: Option<SystemContextEditorInput>,
    pub workflow_instructions: Vec<String>,
}

pub(crate) fn build_session_system_prompt(
    request: SessionSystemPromptBuildRequest<'_>,
) -> Result<SessionSystemPromptBuild, String> {
    let meta = request
        .session_store
        .get_session_meta(request.data_dir, &request.session.id)?
        .ok_or_else(|| format!("Session not found: {}", request.session.id))?;
    let (read_file_paths, agent_read_paths) = cached_or_restored_agent_read_paths(
        request.session_store,
        request.data_dir,
        request.session,
        &meta,
    )?;
    let merged_workflow_instructions =
        merge_workflow_instructions(meta.workflow_instructions, request.workflow_instructions);
    let built = build_system_context(
        request.branch_diff_context,
        request.instruction_source,
        SystemContextBuildRequest {
            worktree_path: &request.session.worktree_path,
            previous_meta: meta.context_epoch.as_ref(),
            backend_id: request.backend_id,
            model_id: request.model_id,
            mentions: request.mentions,
            editor_context: request.editor_context,
            read_file_paths,
            workflow_state: workflow_state_payload(request.session.workflow_node_context.as_ref()),
            workflow_instructions: merged_workflow_instructions.clone(),
        },
    );
    Ok(SessionSystemPromptBuild {
        context_epoch: built.state.to_meta(),
        system_context: built,
        workflow_instructions: merged_workflow_instructions,
        agent_read_paths,
    })
}

pub(crate) fn persist_session_system_prompt_build(
    session_store: &SessionStore,
    data_dir: &Path,
    session_id: &str,
    built: &SessionSystemPromptBuild,
) -> Result<(), String> {
    session_store.update_system_context_private_meta_if_changed(
        data_dir,
        session_id,
        Some(built.context_epoch.clone()),
        built.workflow_instructions.clone(),
        built.agent_read_paths.clone(),
    )?;
    Ok(())
}

fn cached_or_restored_agent_read_paths(
    session_store: &SessionStore,
    data_dir: &Path,
    session: &ChatSession,
    meta: &crate::usecase::agent_session::session::SessionMeta,
) -> Result<(Vec<PathBuf>, Option<Vec<PathBuf>>), String> {
    if let Some(paths) = meta.agent_read_paths.clone() {
        return Ok((paths, None));
    }
    let stored = session_store
        .load_full_session_for_restore(data_dir, &session.id)?
        .ok_or_else(|| format!("Session not found: {}", session.id))?;
    let paths = agent_read_paths_from_messages(&stored.messages);
    Ok((paths.clone(), Some(paths)))
}

fn workflow_state_payload(
    context: Option<&crate::usecase::agent_session::session::WorkflowNodeContextDto>,
) -> Option<String> {
    context.and_then(|context| serde_json::to_string(context).ok())
}

fn merge_workflow_instructions(
    stored: impl IntoIterator<Item = String>,
    additional: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut merged = Vec::new();
    for instruction in stored.into_iter().chain(additional) {
        if instruction.trim().is_empty() || merged.contains(&instruction) {
            continue;
        }
        merged.push(instruction);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::agent_session::FileSessionStorage;
    use crate::usecase::agent_session::session::{
        add_message_internal, create_session_internal, MessagePart, MessageRole,
    };
    use std::sync::Arc;

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

    #[test]
    fn build_only_does_not_persist_context_or_read_path_cache() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let store = SessionStore::new(storage);
        let session =
            create_session_internal(&store, temp.path(), "/repo", Some("claude".to_string()))
                .unwrap();
        std::fs::remove_file(
            temp.path()
                .join("sessions")
                .join(&session.id)
                .join("private_context.json"),
        )
        .unwrap();
        let store = SessionStore::new(Arc::new(FileSessionStorage::default()));
        let session = store
            .get_session_shell(temp.path(), &session.id)
            .unwrap()
            .expect("session shell");

        let built = build_session_system_prompt(SessionSystemPromptBuildRequest {
            session_store: &store,
            data_dir: temp.path(),
            session: &session,
            branch_diff_context: None,
            instruction_source: &EmptyInstructionSource,
            backend_id: "claude",
            model_id: None,
            mentions: &[],
            editor_context: None,
            workflow_instructions: Vec::new(),
        })
        .unwrap();

        assert!(!built.system_context.snapshots.is_empty());
        let meta = store
            .get_session_meta(temp.path(), &session.id)
            .unwrap()
            .expect("meta");
        assert!(meta.context_epoch.is_none());
        assert!(meta.agent_read_paths.is_none());
    }

    #[test]
    fn cache_miss_fallback_persists_read_paths_for_subsequent_builds() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let store = SessionStore::new(storage);
        let session =
            create_session_internal(&store, temp.path(), "/repo", Some("claude".to_string()))
                .unwrap();
        add_message_internal(
            &store,
            temp.path(),
            &session.id,
            MessageRole::Agent,
            "",
            Some(vec![MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({"file_path": "src/local/file.rs"}).into(),
                id: "tool-1".to_string(),
                parent_tool_use_id: None,
            }]),
            None,
        )
        .unwrap();
        std::fs::remove_file(
            temp.path()
                .join("sessions")
                .join(&session.id)
                .join("private_context.json"),
        )
        .unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let store = SessionStore::new(storage.clone());
        let session = store
            .get_session_shell(temp.path(), &session.id)
            .unwrap()
            .expect("session shell");
        storage.reset_message_read_count();

        let first = build_session_system_prompt(SessionSystemPromptBuildRequest {
            session_store: &store,
            data_dir: temp.path(),
            session: &session,
            branch_diff_context: None,
            instruction_source: &EmptyInstructionSource,
            backend_id: "claude",
            model_id: None,
            mentions: &[],
            editor_context: None,
            workflow_instructions: Vec::new(),
        })
        .unwrap();

        assert_eq!(
            first.agent_read_paths,
            Some(vec![PathBuf::from("src/local/file.rs")])
        );
        assert!(storage.message_read_count() > 0);
        persist_session_system_prompt_build(&store, temp.path(), &session.id, &first).unwrap();

        storage.reset_message_read_count();
        let second = build_session_system_prompt(SessionSystemPromptBuildRequest {
            session_store: &store,
            data_dir: temp.path(),
            session: &session,
            branch_diff_context: None,
            instruction_source: &EmptyInstructionSource,
            backend_id: "claude",
            model_id: None,
            mentions: &[],
            editor_context: None,
            workflow_instructions: Vec::new(),
        })
        .unwrap();

        assert_eq!(second.agent_read_paths, None);
        assert_eq!(storage.message_read_count(), 0);
    }

    #[test]
    fn cached_read_paths_do_not_read_message_chunks_per_turn() {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(FileSessionStorage::default());
        let store = SessionStore::new(storage.clone());
        let session =
            create_session_internal(&store, temp.path(), "/repo", Some("claude".to_string()))
                .unwrap();
        for index in 0..25 {
            add_message_internal(
                &store,
                temp.path(),
                &session.id,
                MessageRole::Agent,
                "",
                Some(vec![MessagePart::ToolUse {
                    tool: "Read".to_string(),
                    input: serde_json::json!({"file_path": format!("src/file-{index}.rs")}).into(),
                    id: format!("tool-{index}"),
                    parent_tool_use_id: None,
                }]),
                None,
            )
            .unwrap();
        }
        let session = store
            .get_session_shell(temp.path(), &session.id)
            .unwrap()
            .expect("session shell");
        storage.reset_message_read_count();

        let built = build_session_system_prompt(SessionSystemPromptBuildRequest {
            session_store: &store,
            data_dir: temp.path(),
            session: &session,
            branch_diff_context: None,
            instruction_source: &EmptyInstructionSource,
            backend_id: "claude",
            model_id: None,
            mentions: &[],
            editor_context: None,
            workflow_instructions: Vec::new(),
        })
        .unwrap();

        assert_eq!(built.agent_read_paths, None);
        assert_eq!(storage.message_read_count(), 0);
    }
}
