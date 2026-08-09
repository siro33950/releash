use std::sync::{Arc, Mutex};

use super::{
    ProviderAgentSessionActivityDto, ProviderAgentSessionGarbageCollectionOutcome,
    ProviderAgentSessionGarbageCollectionPort, ProviderAgentSessionItemDto,
    ProviderAgentSessionLifecycleDto, ProviderAgentSessionLifecycleUsecaseError,
    ProviderAgentSessionListPageDto, ProviderAgentSessionListRequest,
    ProviderAgentSessionOperationsDto, ProviderAgentSessionOriginDto,
    ProviderAgentSessionProviderDto, ProviderAgentSessionQueryError,
    ProviderAgentSessionQueryService, ProviderAgentSessionReadUsecase,
};
use crate::domain::agent_session::ProviderAgentTerminalObservationGateway;
use crate::domain::terminal_surface::{TerminalActivity, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;

struct FixedActivityTerminal {
    activity: TerminalActivity,
    queried: Mutex<Vec<TerminalSurfaceOwner>>,
}

impl FixedActivityTerminal {
    fn new(activity: TerminalActivity) -> Self {
        Self {
            activity,
            queried: Mutex::new(Vec::new()),
        }
    }
}

impl ProviderAgentTerminalObservationGateway for FixedActivityTerminal {
    fn owner_for_runtime_generation(
        &self,
        _session_key: &str,
        _runtime_generation: u64,
    ) -> Option<TerminalSurfaceOwner> {
        None
    }

    fn exited_session_owners(&self) -> Vec<(u64, TerminalSurfaceOwner, Option<i32>)> {
        Vec::new()
    }

    fn session_exit_code(&self, _owner: &TerminalSurfaceOwner) -> Option<i32> {
        None
    }

    fn session_activity(&self, owner: &TerminalSurfaceOwner) -> TerminalActivity {
        self.queried.lock().unwrap().push(owner.clone());
        self.activity
    }

    fn session_worktree_path(&self, _session_key: &str) -> Option<String> {
        None
    }
}

struct MutableSessionQuery {
    items: Arc<Mutex<Vec<ProviderAgentSessionItemDto>>>,
}

#[async_trait::async_trait]
impl ProviderAgentSessionQueryService for MutableSessionQuery {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<ProviderAgentSessionItemDto>, ProviderAgentSessionQueryError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .find(|item| item.id == agent_session_id)
            .cloned())
    }

    async fn list(
        &self,
        request: ProviderAgentSessionListRequest,
    ) -> Result<ProviderAgentSessionListPageDto, ProviderAgentSessionQueryError> {
        Ok(ProviderAgentSessionListPageDto {
            items: self
                .items
                .lock()
                .unwrap()
                .iter()
                .filter(|item| item.workspace_identity == request.workspace.as_str())
                .take(request.limit)
                .cloned()
                .collect(),
            next_after_session_id: None,
        })
    }
}

struct RemovingGarbageCollector {
    items: Arc<Mutex<Vec<ProviderAgentSessionItemDto>>>,
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ProviderAgentSessionGarbageCollectionPort for RemovingGarbageCollector {
    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        _caller_request_id: &str,
    ) -> Result<
        ProviderAgentSessionGarbageCollectionOutcome,
        ProviderAgentSessionLifecycleUsecaseError,
    > {
        self.calls
            .lock()
            .unwrap()
            .push(agent_session_id.to_string());
        if agent_session_id == "orphan" {
            self.items
                .lock()
                .unwrap()
                .retain(|item| item.id != agent_session_id);
            return Ok(ProviderAgentSessionGarbageCollectionOutcome::GarbageCollected);
        }
        Ok(ProviderAgentSessionGarbageCollectionOutcome::Retained)
    }
}

fn item(id: &str) -> ProviderAgentSessionItemDto {
    ProviderAgentSessionItemDto {
        id: id.to_string(),
        workspace_identity: "/repo".to_string(),
        worktree_path: "/repo/worktree".to_string(),
        provider: ProviderAgentSessionProviderDto::Claude,
        origin: ProviderAgentSessionOriginDto::Standalone,
        lifecycle: ProviderAgentSessionLifecycleDto::Open,
        provider_session_id: None,
        transcript_ref: None,
        operations: ProviderAgentSessionOperationsDto {
            can_archive: true,
            can_restore: false,
            can_delete: false,
        },
        activity: ProviderAgentSessionActivityDto::Idle,
        last_exit_abnormal: false,
    }
}

#[tokio::test]
async fn test_provider_agent_session_read一覧取得時にgc済みsessionを返さない() {
    let items = Arc::new(Mutex::new(vec![item("orphan"), item("live")]));
    let query = Arc::new(MutableSessionQuery {
        items: items.clone(),
    });
    let collector = Arc::new(RemovingGarbageCollector {
        items,
        calls: Mutex::new(Vec::new()),
    });
    let usecase = ProviderAgentSessionReadUsecase::new(
        query,
        collector.clone(),
        Arc::new(FixedActivityTerminal::new(TerminalActivity::Idle)),
    );

    let page = usecase
        .list(ProviderAgentSessionListRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            lifecycle: None,
            origin: None,
            limit: 100,
            after_session_id: None,
        })
        .await
        .unwrap();

    assert_eq!(
        page.items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["live"]
    );
    assert!(collector
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|id| id == "orphan"));
}

#[tokio::test]
async fn test_provider_agent_session_read単体取得時にgc済みsessionを返さない() {
    let items = Arc::new(Mutex::new(vec![item("orphan")]));
    let query = Arc::new(MutableSessionQuery {
        items: items.clone(),
    });
    let collector = Arc::new(RemovingGarbageCollector {
        items,
        calls: Mutex::new(Vec::new()),
    });
    let usecase = ProviderAgentSessionReadUsecase::new(
        query,
        collector,
        Arc::new(FixedActivityTerminal::new(TerminalActivity::Idle)),
    );

    assert!(usecase.get("orphan").await.unwrap().is_none());
}

#[tokio::test]
async fn test_provider_agent_session_read_open_sessionはterminalのactivity分類を反映する() {
    let mut open_item = item("open-running");
    open_item.lifecycle = ProviderAgentSessionLifecycleDto::Open;
    let mut paused_item = item("paused-idle");
    paused_item.lifecycle = ProviderAgentSessionLifecycleDto::Paused;
    paused_item.last_exit_abnormal = true;
    let items = Arc::new(Mutex::new(vec![open_item, paused_item]));
    let query = Arc::new(MutableSessionQuery {
        items: items.clone(),
    });
    let collector = Arc::new(RemovingGarbageCollector {
        items,
        calls: Mutex::new(Vec::new()),
    });
    let terminal = Arc::new(FixedActivityTerminal::new(TerminalActivity::Running));
    let usecase = ProviderAgentSessionReadUsecase::new(query, collector, terminal.clone());

    let page = usecase
        .list(ProviderAgentSessionListRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            lifecycle: None,
            origin: None,
            limit: 100,
            after_session_id: None,
        })
        .await
        .unwrap();

    assert_eq!(
        page.items
            .iter()
            .map(|item| (item.id.as_str(), item.activity, item.last_exit_abnormal))
            .collect::<Vec<_>>(),
        vec![
            (
                "open-running",
                ProviderAgentSessionActivityDto::Running,
                false
            ),
            ("paused-idle", ProviderAgentSessionActivityDto::Idle, true),
        ]
    );
    assert_eq!(
        terminal.queried.lock().unwrap().as_slice(),
        &[TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "open-running").unwrap()],
        "paused sessionはterminal照会せずidle確定"
    );
}

#[test]
fn test_provider_agent_session_read_一覧itemのjsonにactivityとlastexitabnormalを載せる() {
    let mut serialized_item = item("json-shape");
    serialized_item.activity = ProviderAgentSessionActivityDto::Running;
    serialized_item.last_exit_abnormal = true;
    let json = serde_json::to_value(&serialized_item).unwrap();

    assert_eq!(json["activity"], "running");
    assert_eq!(json["lastExitAbnormal"], true);
}
