use std::sync::{Arc, Mutex};

use super::{
    AgentSessionActivityDto, AgentSessionGarbageCollectionOutcome,
    AgentSessionGarbageCollectionPort, AgentSessionItemDto, AgentSessionLifecycleDto,
    AgentSessionLifecycleUsecaseError, AgentSessionOperationsDto, AgentSessionProviderDto,
    AgentSessionQueryError, AgentSessionQueryService, AgentSessionReadUsecase,
    AgentSessionTreeLocationDto,
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
    items: Arc<Mutex<Vec<AgentSessionItemDto>>>,
}

#[async_trait::async_trait]
impl AgentSessionQueryService for MutableSessionQuery {
    async fn get(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .find(|item| item.id == agent_session_id)
            .cloned())
    }
}

struct RemovingGarbageCollector {
    items: Arc<Mutex<Vec<AgentSessionItemDto>>>,
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl AgentSessionGarbageCollectionPort for RemovingGarbageCollector {
    async fn reconcile_garbage_collection(
        &self,
        agent_session_id: &str,
        _caller_request_id: &str,
    ) -> Result<AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecaseError> {
        self.calls
            .lock()
            .unwrap()
            .push(agent_session_id.to_string());
        if agent_session_id == "orphan" {
            self.items
                .lock()
                .unwrap()
                .retain(|item| item.id != agent_session_id);
            return Ok(AgentSessionGarbageCollectionOutcome::GarbageCollected);
        }
        Ok(AgentSessionGarbageCollectionOutcome::Retained)
    }
}

fn item(id: &str) -> AgentSessionItemDto {
    AgentSessionItemDto {
        id: id.to_string(),
        workspace_identity: "/repo".to_string(),
        worktree_path: "/repo/worktree".to_string(),
        provider: AgentSessionProviderDto::Claude,
        tree_location: AgentSessionTreeLocationDto {
            tree_id: id.to_string(),
            node_execution_id: id.to_string(),
        },
        lifecycle: AgentSessionLifecycleDto::Open,
        provider_session_id: None,
        transcript_ref: None,
        operations: AgentSessionOperationsDto {
            can_archive: true,
            can_restore: false,
            can_delete: false,
            can_resume: false,
        },
        activity: AgentSessionActivityDto::Idle,
        last_exit_abnormal: false,
    }
}

#[tokio::test]
async fn test_agent_session_read単体取得時にgc済みsessionを返さない() {
    let items = Arc::new(Mutex::new(vec![item("orphan")]));
    let query = Arc::new(MutableSessionQuery {
        items: items.clone(),
    });
    let collector = Arc::new(RemovingGarbageCollector {
        items,
        calls: Mutex::new(Vec::new()),
    });
    let usecase = AgentSessionReadUsecase::new(
        query,
        collector,
        Arc::new(FixedActivityTerminal::new(TerminalActivity::Idle)),
    );

    assert!(usecase.get("orphan").await.unwrap().is_none());
}

#[tokio::test]
async fn test_agent_session_read_open_sessionはterminalのactivity分類を反映する() {
    let mut open_item = item("open-running");
    open_item.lifecycle = AgentSessionLifecycleDto::Open;
    let mut paused_item = item("paused-idle");
    paused_item.lifecycle = AgentSessionLifecycleDto::Paused;
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
    let usecase = AgentSessionReadUsecase::new(query, collector, terminal.clone());

    let open = usecase.get("open-running").await.unwrap().unwrap();
    let paused = usecase.get("paused-idle").await.unwrap().unwrap();
    assert_eq!(
        [&open, &paused]
            .into_iter()
            .map(|item| (item.id.as_str(), item.activity, item.last_exit_abnormal))
            .collect::<Vec<_>>(),
        vec![
            ("open-running", AgentSessionActivityDto::Running, false),
            ("paused-idle", AgentSessionActivityDto::Idle, true),
        ]
    );
    assert_eq!(
        terminal.queried.lock().unwrap().as_slice(),
        &[TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "open-running").unwrap()],
        "paused sessionはterminal照会せずidle確定"
    );
}

#[test]
fn test_agent_session_read_一覧itemのjsonにactivityとlastexitabnormalを載せる() {
    let mut serialized_item = item("json-shape");
    serialized_item.activity = AgentSessionActivityDto::Running;
    serialized_item.last_exit_abnormal = true;
    let json = serde_json::to_value(&serialized_item).unwrap();

    assert_eq!(json["activity"], "running");
    assert_eq!(json["lastExitAbnormal"], true);
}
