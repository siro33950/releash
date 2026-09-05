use std::sync::{Arc, Mutex};

use super::{
    AgentSessionChangeNotifier, AgentSessionRenameError, AgentSessionRenameExecutor,
    AgentSessionRenameUsecase,
};
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycleEvent, AgentSessionMutationOutcome,
    AgentSessionRemovalAuthorization, AgentSessionTreeLocation,
};
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::provider_lifecycle::{ProviderKind, ScopedProviderLifecycleEvent};
use crate::domain::workspace_tree::WorkspaceIdentity;

struct RenameRepository {
    session: Mutex<Option<VersionedAgentSession>>,
    saves: Mutex<usize>,
}

#[async_trait::async_trait]
impl AgentSessionRepository for RenameRepository {
    async fn create(
        &self,
        _session: AgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::InvalidRequest)
    }

    async fn create_with_lifecycle_events(
        &self,
        _session: AgentSession,
        _lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::InvalidRequest)
    }

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionRepositoryError> {
        Ok(self
            .session
            .lock()
            .unwrap()
            .as_ref()
            .filter(|session| session.session().id() == session_id)
            .cloned())
    }

    async fn save(
        &self,
        session: VersionedAgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        let revision = session.revision();
        let mut session = session.into_session();
        if !matches!(
            session.take_uncommitted_events().as_slice(),
            [AgentSessionLifecycleEvent::SessionNodeRenamed { .. }]
        ) {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        }
        *self.saves.lock().unwrap() += 1;
        let saved = VersionedAgentSession::restored(session, revision.saturating_add(1));
        *self.session.lock().unwrap() = Some(saved.clone());
        Ok(saved)
    }

    async fn remove(
        &self,
        _session: VersionedAgentSession,
        _authorization: AgentSessionRemovalAuthorization,
        _caller_request_id: &str,
    ) -> Result<(), AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::InvalidRequest)
    }
}

#[derive(Default)]
struct RenameNotifier {
    worktrees: Mutex<Vec<String>>,
}

impl AgentSessionChangeNotifier for RenameNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
        self.worktrees
            .lock()
            .unwrap()
            .push(worktree_path.to_string());
    }
}

fn context() -> (
    Arc<RenameRepository>,
    Arc<RenameNotifier>,
    AgentSessionRenameUsecase,
) {
    let mut session = AgentSession::create(
        "rename-session",
        WorkspaceIdentity::new("workspace"),
        "/repo/worktree",
        ProviderKind::Claude,
        AgentSessionTreeLocation::session_tree_root("rename-session").unwrap(),
    )
    .unwrap();
    session.take_uncommitted_events();
    let repository = Arc::new(RenameRepository {
        session: Mutex::new(Some(VersionedAgentSession::restored(session, 1))),
        saves: Mutex::new(0),
    });
    let notifier = Arc::new(RenameNotifier::default());
    let usecase = AgentSessionRenameUsecase::new(repository.clone(), notifier.clone());
    (repository, notifier, usecase)
}

#[tokio::test]
async fn test_agent_session_rename_usecase_前後空白を落として保存しworktree通知する() {
    let (repository, notifier, usecase) = context();

    let outcome = usecase
        .rename("rename-session", "  release review  ")
        .await
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::Applied);
    assert_eq!(*repository.saves.lock().unwrap(), 1);
    assert_eq!(
        repository
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .session()
            .manual_name(),
        Some("release review")
    );
    assert_eq!(
        notifier.worktrees.lock().unwrap().as_slice(),
        &["/repo/worktree".to_string()]
    );
}

#[tokio::test]
async fn test_agent_session_rename_usecase_空白だけの名前を拒否し保存も通知もしない() {
    let (repository, notifier, usecase) = context();

    let result = usecase.rename("rename-session", " \t ").await;

    assert_eq!(result, Err(AgentSessionRenameError::InvalidOperation));
    assert_eq!(*repository.saves.lock().unwrap(), 0);
    assert!(notifier.worktrees.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_agent_session_rename_usecase_同じ名前なら保存も通知もしない() {
    let (repository, notifier, usecase) = context();
    usecase
        .rename("rename-session", "stable name")
        .await
        .unwrap();
    *repository.saves.lock().unwrap() = 0;
    notifier.worktrees.lock().unwrap().clear();

    let outcome = usecase
        .rename("rename-session", "stable name")
        .await
        .unwrap();

    assert_eq!(outcome, AgentSessionMutationOutcome::AlreadyApplied);
    assert_eq!(*repository.saves.lock().unwrap(), 0);
    assert!(notifier.worktrees.lock().unwrap().is_empty());
}
