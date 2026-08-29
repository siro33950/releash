use std::sync::{Arc, Mutex};

use super::{
    AgentSessionGarbageCollectionOutcome, AgentSessionGarbageCollectionPort, AgentSessionItemDto,
    AgentSessionLifecycleDto, AgentSessionLifecycleUsecaseError, AgentSessionOperationsDto,
    AgentSessionProviderDto, AgentSessionQueryError, AgentSessionQueryService,
    AgentSessionReadUsecase, AgentSessionTreeLocationDto,
};

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
    let usecase = AgentSessionReadUsecase::new(query, collector);

    assert!(usecase.get("orphan").await.unwrap().is_none());
}

#[tokio::test]
async fn test_agent_session_read_lifecycleと異常終了状態をquery結果のまま返す() {
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
    let usecase = AgentSessionReadUsecase::new(query, collector);

    let open = usecase.get("open-running").await.unwrap().unwrap();
    let paused = usecase.get("paused-idle").await.unwrap().unwrap();
    assert_eq!(
        [
            (open.id.as_str(), open.lifecycle, open.last_exit_abnormal),
            (
                paused.id.as_str(),
                paused.lifecycle,
                paused.last_exit_abnormal,
            ),
        ],
        [
            ("open-running", AgentSessionLifecycleDto::Open, false),
            ("paused-idle", AgentSessionLifecycleDto::Paused, true),
        ]
    );
}

#[test]
fn test_agent_session_read_一覧itemのjsonからactivityを除きlastexitabnormalを維持する() {
    let mut serialized_item = item("json-shape");
    serialized_item.last_exit_abnormal = true;
    let json = serde_json::to_value(&serialized_item).unwrap();

    assert!(json.get("activity").is_none());
    assert_eq!(json["lastExitAbnormal"], true);
}
