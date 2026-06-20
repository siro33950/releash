use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use crate::adaptor::protocol::workflow::WorkflowStateView;
use crate::agent_message_dispatcher::{
    dispatch_agent_message, AgentMessageDispatchContext, AgentMessageDispatchRequest,
};
use crate::infrastructure::agent_session::runtime::{
    AgentBackendRegistry, AgentProcessMap, ImageAttachment, SendMessageResponse,
};
use crate::infrastructure::agent_session::runtime_gateway::AgentRuntimeGateway;
use crate::usecase::agent_session::session::{ChatSession, OpenTabRegistry, SessionStore};
use crate::usecase::workflow::step_lifecycle::{
    ResolvedWorkflowStepSession, WorkflowStepLifecycleError,
};
use crate::usecase::workflow::{WorkflowRuntimeUsecase, WorkflowStepLifecycleUsecase};

pub(crate) type AgentProcessMapState = Arc<Mutex<AgentProcessMap>>;
pub(crate) type AgentBackendRegistryState = Arc<AgentBackendRegistry>;
pub(crate) type AgentImageAttachment = ImageAttachment;
pub(crate) type AgentSendMessageResponse = SendMessageResponse;
pub(crate) type OpenTabRegistryState = Arc<OpenTabRegistry>;
pub(crate) type SessionStoreState = Arc<SessionStore>;
pub(crate) type WorkflowStepLifecycleUsecaseState = Arc<WorkflowStepLifecycleUsecase>;

pub(crate) fn normalize_path(path: &std::path::Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}

pub(crate) fn validate_relative_path(path: &str, repo_root: &str) -> Result<PathBuf, String> {
    if std::path::Path::new(path).is_absolute() {
        return Err("絶対パスは拒否されます".to_string());
    }
    let root = std::path::Path::new(repo_root)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let resolved = normalize_path(&root.join(path));
    if !resolved.starts_with(&root) {
        return Err("プロジェクトルート外のパスは拒否されます".to_string());
    }
    // Canonicalize the deepest existing ancestor to catch symlinks even when
    // the file itself does not exist yet.
    let mut check = resolved.as_path();
    loop {
        if check.exists() {
            let canonical = check.canonicalize().map_err(|e| e.to_string())?;
            if !canonical.starts_with(&root) {
                return Err(
                    "シンボリックリンクによるプロジェクトルート外へのアクセスは拒否されます"
                        .to_string(),
                );
            }
            break;
        }
        match check.parent() {
            Some(parent) => check = parent,
            None => break,
        }
    }
    Ok(resolved)
}

pub(crate) async fn build_workflow_state_view(
    state: crate::domain::workflow::WorkflowStateSnapshot,
    handles: &AgentProcessMapState,
    open_tabs: &OpenTabRegistryState,
) -> WorkflowStateView {
    crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
        state, handles, open_tabs,
    )
    .await
}

pub(crate) async fn dispatch_agent_message_with_runtime(
    app: &tauri::AppHandle,
    session_store: &SessionStoreState,
    registry: &AgentBackendRegistryState,
    handles: &AgentProcessMapState,
    req: AgentMessageDispatchRequest,
) -> Result<AgentSendMessageResponse, String> {
    dispatch_agent_message(
        AgentMessageDispatchContext {
            gateway: AgentRuntimeGateway {
                app,
                session_store,
                registry,
                handles,
            },
        },
        req,
    )
    .await
}

pub(crate) async fn open_workflow_step_tab(
    step_lifecycle: &WorkflowStepLifecycleUsecaseState,
    chat_session_id: &str,
) -> Result<ResolvedWorkflowStepSession, WorkflowStepLifecycleError> {
    step_lifecycle.open_tab(chat_session_id).await
}

pub(crate) async fn emit_after_workflow_step_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session: &ChatSession,
    handles: &AgentProcessMapState,
    open_tabs: &OpenTabRegistryState,
) {
    let Some(runtime) = app
        .try_state::<Arc<WorkflowRuntimeUsecase>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    let Ok(Some(state)) = runtime.get_state_by_worktree(&session.worktree_path).await else {
        return;
    };
    crate::adaptor::gateway::workflow::emit_workflow_state_from_snapshot(
        app,
        &session.worktree_path,
        state,
        handles,
        open_tabs,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_relative_path_rejects_absolute() {
        let result = validate_relative_path("/etc/passwd", "/tmp");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = validate_relative_path("../../etc/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_rejects_nested_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("foo")).unwrap();
        let result = validate_relative_path("foo/../../etc/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_accepts_valid_subdir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let result = validate_relative_path("src/main.rs", dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_relative_path_rejects_symlink_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("inner")).unwrap();
        std::os::unix::fs::symlink("/etc", dir.path().join("inner/secret_link")).unwrap();
        let result =
            validate_relative_path("inner/secret_link/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("シンボリックリンク"));
    }

    #[test]
    fn test_normalize_path_removes_parent_dir() {
        let path = std::path::Path::new("/home/user/../etc/passwd");
        let normalized = normalize_path(path);
        assert_eq!(normalized, std::path::PathBuf::from("/home/etc/passwd"));
    }

    #[test]
    fn test_normalize_path_removes_cur_dir() {
        let path = std::path::Path::new("/home/./user/./file.txt");
        let normalized = normalize_path(path);
        assert_eq!(normalized, std::path::PathBuf::from("/home/user/file.txt"));
    }
}

pub(crate) async fn emit_workflow_step_target_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: &ResolvedWorkflowStepSession,
    handles: &AgentProcessMapState,
    open_tabs: &OpenTabRegistryState,
) {
    let Some(runtime) = app
        .try_state::<Arc<WorkflowRuntimeUsecase>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    let Ok(Some(state)) = runtime.get_state_by_worktree(&target.worktree_path).await else {
        return;
    };
    crate::adaptor::gateway::workflow::emit_workflow_state_from_snapshot(
        app,
        &target.worktree_path,
        state,
        handles,
        open_tabs,
    )
    .await;
}
