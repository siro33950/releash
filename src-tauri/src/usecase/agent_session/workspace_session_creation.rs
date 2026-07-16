use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::domain::agent_session::PermissionMode;
use crate::domain::repository::normalize_repo_path;
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::session::{
    create_session_with_resolved_options_and_id, ChatSession, SessionStore,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionCreationRequest {
    pub worktree_path: String,
    pub permission_mode: String,
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceSessionCreationRequest {
    pub request_id: String,
    pub session: SessionCreationRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedCreationOptions {
    permission_mode: PermissionMode,
    backend_id: String,
    model_id: String,
}

pub(crate) trait SessionCreationOptionsResolver {
    fn resolve_model_entry(&self, model_id: &str) -> Result<(String, String), String>;
    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String>;
    fn default_model_for(&self, backend_id: &str) -> Result<String, String>;
}

impl SessionCreationOptionsResolver for AgentBackendRegistry {
    fn resolve_model_entry(&self, model_id: &str) -> Result<(String, String), String> {
        let model = AgentBackendRegistry::resolve_model_entry(self, model_id)?;
        Ok((model.backend, model.model_id))
    }

    fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
        AgentBackendRegistry::resolve_backend_id(self, backend_id)
    }

    fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
        AgentBackendRegistry::default_model_for(self, backend_id)
    }
}

/// Owns standalone Workspace session creation, including durable request idempotency.
///
/// The request UUID is also the persisted session ID. The process-local lock closes the
/// check-then-save race, while the persisted session makes the same request recoverable after a
/// remount or app restart. The lock is deliberately shared across request IDs: creation is a cold
/// path, and serializing the short metadata commit avoids an unbounded per-request lock registry.
pub(crate) struct WorkspaceSessionCreationUsecase {
    session_store: Arc<SessionStore>,
    creation_lock: Mutex<()>,
}

impl WorkspaceSessionCreationUsecase {
    pub(crate) fn new(session_store: Arc<SessionStore>) -> Self {
        Self {
            session_store,
            creation_lock: Mutex::new(()),
        }
    }

    /// Legacy, non-idempotent creation entry point. Keeping it on the same usecase guarantees that
    /// the existing command and the Workspace-specific command resolve backend/model options with
    /// identical semantics.
    pub(crate) fn create_session(
        &self,
        resolver: &impl SessionCreationOptionsResolver,
        data_dir: &Path,
        request: SessionCreationRequest,
    ) -> Result<ChatSession, String> {
        let options = resolve_creation_options(resolver, &request)?;
        create_session_with_resolved_options_and_id(
            &self.session_store,
            data_dir,
            uuid::Uuid::new_v4().to_string(),
            &request.worktree_path,
            options.backend_id,
            options.permission_mode,
            options.model_id,
            false,
        )
    }

    pub(crate) fn create_workspace_session(
        &self,
        resolver: &impl SessionCreationOptionsResolver,
        data_dir: &Path,
        request: WorkspaceSessionCreationRequest,
    ) -> Result<String, String> {
        let request_id = uuid::Uuid::parse_str(&request.request_id)
            .map_err(|_| "workspace session request_id must be a UUID".to_string())?
            .to_string();

        let _creation_guard = self.creation_lock.lock();
        if let Some(existing) = self
            .session_store
            .get_session_shell(data_dir, &request_id)?
        {
            ensure_matching_request(&existing, &request.session.worktree_path)?;
            return Ok(request_id);
        }

        let options = resolve_creation_options(resolver, &request.session)?;
        create_session_with_resolved_options_and_id(
            &self.session_store,
            data_dir,
            request_id.clone(),
            &request.session.worktree_path,
            options.backend_id,
            options.permission_mode,
            options.model_id,
            false,
        )?;
        Ok(request_id)
    }
}

fn resolve_creation_options(
    resolver: &impl SessionCreationOptionsResolver,
    request: &SessionCreationRequest,
) -> Result<ResolvedCreationOptions, String> {
    let permission_mode =
        PermissionMode::parse(&request.permission_mode).map_err(|error| error.to_string())?;
    let resolved_model = request
        .model_id
        .as_deref()
        .map(|model_id| resolver.resolve_model_entry(model_id))
        .transpose()?;
    let backend_id = resolver.resolve_backend_id(
        resolved_model
            .as_ref()
            .map(|(backend_id, _)| backend_id.clone())
            .or_else(|| request.backend_id.clone()),
    )?;
    let model_id = match resolved_model {
        Some((_, model_id)) => model_id,
        None => resolver.default_model_for(&backend_id)?,
    };
    Ok(ResolvedCreationOptions {
        permission_mode,
        backend_id,
        model_id,
    })
}

fn ensure_matching_request(existing: &ChatSession, worktree_path: &str) -> Result<(), String> {
    let matches = existing.worktree_path == normalize_repo_path(worktree_path)
        && !existing.plan_mode
        && !existing.workflow_node_session
        && existing.workflow_node_context.is_none();
    if matches {
        Ok(())
    } else {
        Err(format!(
            "workspace session request {} already exists outside the requested standalone Workspace",
            existing.id
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    const REQUEST_ID: &str = "f46f92ee-5a12-4be9-8ef3-e0a32fd254a0";

    #[derive(Debug, Default)]
    struct TestResolver;

    impl SessionCreationOptionsResolver for TestResolver {
        fn resolve_model_entry(&self, model_id: &str) -> Result<(String, String), String> {
            match model_id {
                "claude:sonnet" | "sonnet" => Ok(("claude".to_string(), "sonnet".to_string())),
                "codex:gpt-5" | "gpt-5" => Ok(("codex".to_string(), "gpt-5".to_string())),
                _ => Err(format!("unknown model: {model_id}")),
            }
        }

        fn resolve_backend_id(&self, backend_id: Option<String>) -> Result<String, String> {
            let backend_id = backend_id.unwrap_or_else(|| "claude".to_string());
            match backend_id.as_str() {
                "claude" | "codex" => Ok(backend_id),
                _ => Err(format!("unknown backend: {backend_id}")),
            }
        }

        fn default_model_for(&self, backend_id: &str) -> Result<String, String> {
            match backend_id {
                "claude" => Ok("sonnet".to_string()),
                "codex" => Ok("gpt-5".to_string()),
                _ => Err(format!("unknown backend: {backend_id}")),
            }
        }
    }

    fn session_request() -> SessionCreationRequest {
        SessionCreationRequest {
            worktree_path: "/repo/worktree".to_string(),
            permission_mode: "edit".to_string(),
            backend_id: None,
            model_id: None,
        }
    }

    fn workspace_request() -> WorkspaceSessionCreationRequest {
        WorkspaceSessionCreationRequest {
            request_id: REQUEST_ID.to_string(),
            session: session_request(),
        }
    }

    #[test]
    fn repeated_request_returns_the_same_persisted_session() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let usecase = WorkspaceSessionCreationUsecase::new(store.clone());

        let first = usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap();
        let mut normalized_retry = workspace_request();
        normalized_retry.session.worktree_path = "/repo//worktree/".to_string();
        let second = usecase
            .create_workspace_session(&TestResolver, data_dir.path(), normalized_retry)
            .unwrap();

        assert_eq!(first, REQUEST_ID);
        assert_eq!(second, first);
        let stored = store
            .get_session_shell(data_dir.path(), REQUEST_ID)
            .unwrap()
            .unwrap();
        assert_eq!(stored.id, REQUEST_ID);
        assert_eq!(stored.worktree_path, "/repo/worktree");
        assert_eq!(stored.backend_id.as_deref(), Some("claude"));
        assert_eq!(stored.selected_model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn same_request_id_rejects_a_different_worktree() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let usecase = WorkspaceSessionCreationUsecase::new(store);
        usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap();

        let mut different_worktree = workspace_request();
        different_worktree.session.worktree_path = "/repo/other".to_string();
        let error = usecase
            .create_workspace_session(&TestResolver, data_dir.path(), different_worktree)
            .unwrap_err();

        assert!(error.contains("outside the requested standalone Workspace"));
    }

    #[test]
    fn persisted_request_returns_canonical_session_after_creation_options_change() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let usecase = WorkspaceSessionCreationUsecase::new(store.clone());
        usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap();

        let mut retry = workspace_request();
        retry.session.permission_mode = "full".to_string();
        retry.session.backend_id = Some("codex".to_string());
        retry.session.model_id = Some("codex:gpt-5".to_string());

        let session_id = usecase
            .create_workspace_session(&TestResolver, data_dir.path(), retry)
            .unwrap();

        assert_eq!(session_id, REQUEST_ID);
        let stored = store
            .get_session_shell(data_dir.path(), REQUEST_ID)
            .unwrap()
            .unwrap();
        assert_eq!(stored.permission_mode, "edit");
        assert_eq!(stored.backend_id.as_deref(), Some("claude"));
        assert_eq!(stored.selected_model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn persisted_request_is_recovered_before_retry_options_are_resolved() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let usecase = WorkspaceSessionCreationUsecase::new(store);
        usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap();

        let mut retry = workspace_request();
        retry.session.permission_mode = "retired-mode".to_string();
        retry.session.backend_id = Some("retired-backend".to_string());
        retry.session.model_id = Some("retired-model".to_string());

        let session_id = usecase
            .create_workspace_session(&TestResolver, data_dir.path(), retry)
            .unwrap();

        assert_eq!(session_id, REQUEST_ID);
    }

    #[test]
    fn explicit_model_keeps_the_legacy_model_over_backend_precedence() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let usecase = WorkspaceSessionCreationUsecase::new(store);
        let request = SessionCreationRequest {
            backend_id: Some("claude".to_string()),
            model_id: Some("codex:gpt-5".to_string()),
            ..session_request()
        };

        let session = usecase
            .create_session(&TestResolver, data_dir.path(), request)
            .unwrap();

        assert_eq!(session.backend_id.as_deref(), Some("codex"));
        assert_eq!(session.selected_model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn concurrent_retries_commit_only_once() {
        let data_dir = tempfile::tempdir().unwrap();
        let data_dir = data_dir.path().to_path_buf();
        let store = Arc::new(crate::test_support::build_session_store());
        let save_count = Arc::new(AtomicUsize::new(0));
        store.set_save_hook_for_test({
            let save_count = save_count.clone();
            Arc::new(move |_| {
                save_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });
        let usecase = Arc::new(WorkspaceSessionCreationUsecase::new(store));
        let resolver = Arc::new(TestResolver);
        let barrier = Arc::new(Barrier::new(8));

        let handles = (0..8)
            .map(|_| {
                let usecase = usecase.clone();
                let resolver = resolver.clone();
                let barrier = barrier.clone();
                let data_dir = data_dir.clone();
                thread::spawn(move || {
                    barrier.wait();
                    usecase.create_workspace_session(
                        resolver.as_ref(),
                        &data_dir,
                        workspace_request(),
                    )
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            assert_eq!(handle.join().unwrap().unwrap(), REQUEST_ID);
        }
        assert_eq!(save_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn persisted_request_is_idempotent_after_usecase_restart() {
        let data_dir = tempfile::tempdir().unwrap();
        let first_store = Arc::new(crate::test_support::build_session_store());
        let first_usecase = WorkspaceSessionCreationUsecase::new(first_store);
        first_usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap();

        let restarted_store = Arc::new(crate::test_support::build_session_store());
        let restarted_save_count = Arc::new(AtomicUsize::new(0));
        restarted_store.set_save_hook_for_test({
            let restarted_save_count = restarted_save_count.clone();
            Arc::new(move |_| {
                restarted_save_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });
        let restarted_usecase = WorkspaceSessionCreationUsecase::new(restarted_store);

        let result = restarted_usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap();

        assert_eq!(result, REQUEST_ID);
        assert_eq!(restarted_save_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failed_save_leaves_the_request_retryable() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::test_support::build_session_store());
        let save_attempts = Arc::new(AtomicUsize::new(0));
        store.set_save_hook_for_test({
            let save_attempts = save_attempts.clone();
            Arc::new(move |_| {
                if save_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("injected save failure".to_string())
                } else {
                    Ok(())
                }
            })
        });
        let usecase = WorkspaceSessionCreationUsecase::new(store);

        let first_error = usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap_err();
        let retry = usecase
            .create_workspace_session(&TestResolver, data_dir.path(), workspace_request())
            .unwrap();

        assert_eq!(first_error, "injected save failure");
        assert_eq!(retry, REQUEST_ID);
        assert_eq!(save_attempts.load(Ordering::SeqCst), 2);
    }
}
