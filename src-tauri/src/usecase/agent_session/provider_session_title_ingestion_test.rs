use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{AgentSessionChangeNotifier, ProviderSessionTitleIngestionUsecase};
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycleEvent, AgentSessionRemovalAuthorization,
    AgentSessionTreeLocation,
};
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::agent_session::{
    ProviderSessionTitleGateway, ProviderSessionTitleGatewayError, ProviderSessionTitleRequest,
};
use crate::domain::local_event::WorkflowExecutionMetadataRecord;
use crate::domain::provider_lifecycle::{ProviderKind, ScopedProviderLifecycleEvent};
use crate::domain::workflow::services::fact_replay::{derive_read_model, fold_execution_tree};
use crate::domain::workflow::{
    NodeFact, NodeFactRecord, ProviderSessionTitleObservedFact, SessionExecutionTreeRootFacts,
};
use crate::domain::workspace_tree::{
    runtime_snapshot_nodes, RuntimeSnapshotNodeProjection, WorkspaceIdentity,
};

struct RecordingRepository {
    sessions: Mutex<Vec<VersionedAgentSession>>,
    saved_titles: Mutex<Vec<(String, String)>>,
}

impl RecordingRepository {
    fn new(sessions: Vec<VersionedAgentSession>) -> Self {
        Self {
            sessions: Mutex::new(sessions),
            saved_titles: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl AgentSessionRepository for RecordingRepository {
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
        _session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::InvalidRequest)
    }

    async fn list_open_for_provider_session_title(
        &self,
    ) -> Result<Vec<VersionedAgentSession>, AgentSessionRepositoryError> {
        Ok(self.sessions.lock().unwrap().clone())
    }

    async fn save(
        &self,
        _session: VersionedAgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::InvalidRequest)
    }

    async fn save_provider_session_title(
        &self,
        session: VersionedAgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        let revision = session.revision();
        let mut session = session.into_session();
        let events = session.take_uncommitted_events();
        let [AgentSessionLifecycleEvent::ProviderSessionTitleObserved { title }] =
            events.as_slice()
        else {
            return Err(AgentSessionRepositoryError::InvalidRequest);
        };
        self.saved_titles
            .lock()
            .unwrap()
            .push((session.id().to_string(), title.as_str().to_string()));
        let saved = VersionedAgentSession::restored(session, revision.saturating_add(1));
        let mut sessions = self.sessions.lock().unwrap();
        let existing = sessions
            .iter_mut()
            .find(|candidate| candidate.session().id() == saved.session().id())
            .ok_or(AgentSessionRepositoryError::Conflict)?;
        *existing = saved.clone();
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

struct FixedTitleGateway {
    titles: Mutex<HashMap<String, Result<Option<String>, ProviderSessionTitleGatewayError>>>,
    reads: Mutex<HashMap<String, usize>>,
}

impl FixedTitleGateway {
    fn new(
        titles: impl IntoIterator<
            Item = (
                &'static str,
                Result<Option<&'static str>, ProviderSessionTitleGatewayError>,
            ),
        >,
    ) -> Self {
        Self {
            titles: Mutex::new(
                titles
                    .into_iter()
                    .map(|(id, title)| {
                        (id.to_string(), title.map(|title| title.map(str::to_string)))
                    })
                    .collect(),
            ),
            reads: Mutex::new(HashMap::new()),
        }
    }

    fn set_title(&self, provider_session_id: &str, title: Option<&str>) {
        self.titles.lock().unwrap().insert(
            provider_session_id.to_string(),
            Ok(title.map(str::to_string)),
        );
    }

    fn read_count(&self, provider_session_id: &str) -> usize {
        self.reads
            .lock()
            .unwrap()
            .get(provider_session_id)
            .copied()
            .unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl ProviderSessionTitleGateway for FixedTitleGateway {
    async fn read_title(
        &self,
        request: ProviderSessionTitleRequest,
    ) -> Result<Option<String>, ProviderSessionTitleGatewayError> {
        *self
            .reads
            .lock()
            .unwrap()
            .entry(request.provider_session_id.clone())
            .or_insert(0) += 1;
        self.titles
            .lock()
            .unwrap()
            .get(&request.provider_session_id)
            .cloned()
            .unwrap_or(Ok(None))
    }
}

#[derive(Default)]
struct RecordingNotifier {
    worktrees: Mutex<Vec<String>>,
}

impl AgentSessionChangeNotifier for RecordingNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
        self.worktrees
            .lock()
            .unwrap()
            .push(worktree_path.to_string());
    }
}

fn session(id: &str, provider_session_id: &str, title: Option<&str>) -> VersionedAgentSession {
    let mut session = AgentSession::create(
        id,
        WorkspaceIdentity::new("workspace"),
        format!("/repo/{id}"),
        ProviderKind::Claude,
        AgentSessionTreeLocation::session_tree_root(id).unwrap(),
    )
    .unwrap();
    session.take_uncommitted_events();
    session
        .associate_provider_session(provider_session_id, None)
        .unwrap();
    session.take_uncommitted_events();
    if let Some(title) = title {
        session.observe_provider_session_title(title).unwrap();
        session.take_uncommitted_events();
    }
    VersionedAgentSession::restored(session, 1)
}

fn standalone_session_title(tree_id: &str, records: &[NodeFactRecord]) -> String {
    let folded = fold_execution_tree(tree_id, records).unwrap().unwrap();
    let model = derive_read_model(&folded);
    let execution = WorkflowExecutionMetadataRecord {
        execution_id: model.id,
        workflow_name: model.workflow_name,
        status: model.status,
        worktree_path: model.worktree_path,
        current_node: model.current_node,
        created_from: model.created_from,
        started_at_bits: model.started_at.to_bits(),
        updated_at_bits: model.updated_at.to_bits(),
        completed_at_bits: model.completed_at.map(f64::to_bits),
        error_reason: model.error_reason,
        interruption_reason: model.interruption_reason,
        resume_from_node: model.resume_from_node,
        total_token_usage: model.total_token_usage,
    };
    runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
        execution_id: &folded.aggregate.id,
        workflow_name: &folded.aggregate.workflow.name,
        workspace_identity: &folded.root.workspace_identity,
        workflow_definition: &folded.aggregate.workflow,
        node_executions: &folded.aggregate.node_executions,
        retry_predecessors: &folded.aggregate.retry_predecessors,
        accepts_explicit_retry: folded.aggregate.accepts_explicit_retry(),
        started_at: folded.aggregate.started_at,
        updated_at: folded.aggregate.updated_at,
        execution: &execution,
        recovery_owner_reason: None,
        node_recovery_reasons: &[],
        session_activities: &folded.session_activities,
        session_display_names: &folded.session_display_names,
    })
    .unwrap()
    .into_iter()
    .find(|node| node.node_execution_id.as_deref() == Some(tree_id))
    .unwrap()
    .title
}

#[tokio::test]
async fn test_provider_session_title_ingestion_未取得は毎tickで取得済みは15tickごとに読む() {
    let repository = Arc::new(RecordingRepository::new(vec![
        session("unknown", "provider-unknown", None),
        session("known", "provider-known", Some("Known title")),
    ]));
    let gateway = Arc::new(FixedTitleGateway::new([
        ("provider-unknown", Ok(None)),
        ("provider-known", Ok(Some("Known title"))),
    ]));
    let notifier = Arc::new(RecordingNotifier::default());
    let usecase = ProviderSessionTitleIngestionUsecase::new(repository, gateway.clone(), notifier);

    for _ in 0..16 {
        usecase.ingest_due().await;
    }

    assert_eq!(gateway.read_count("provider-unknown"), 16);
    assert_eq!(gateway.read_count("provider-known"), 2);
}

#[tokio::test]
async fn test_provider_session_title_ingestion_同値なら保存も通知もしない() {
    let repository = Arc::new(RecordingRepository::new(vec![session(
        "same",
        "provider-same",
        Some("Same title"),
    )]));
    let gateway = Arc::new(FixedTitleGateway::new([(
        "provider-same",
        Ok(Some("Same title")),
    )]));
    let notifier = Arc::new(RecordingNotifier::default());
    let usecase =
        ProviderSessionTitleIngestionUsecase::new(repository.clone(), gateway, notifier.clone());

    usecase.ingest_due().await;

    assert!(repository.saved_titles.lock().unwrap().is_empty());
    assert!(notifier.worktrees.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_provider_session_title_ingestion_変化時だけ保存してworktree通知を出す() {
    let repository = Arc::new(RecordingRepository::new(vec![session(
        "changed",
        "provider-changed",
        Some("Old title"),
    )]));
    let gateway = Arc::new(FixedTitleGateway::new([(
        "provider-changed",
        Ok(Some("Old title")),
    )]));
    let notifier = Arc::new(RecordingNotifier::default());
    let usecase = ProviderSessionTitleIngestionUsecase::new(
        repository.clone(),
        gateway.clone(),
        notifier.clone(),
    );
    usecase.ingest_due().await;
    gateway.set_title("provider-changed", Some("Updated title"));

    for _ in 0..15 {
        usecase.ingest_due().await;
    }

    assert_eq!(
        repository.saved_titles.lock().unwrap().as_slice(),
        &[("changed".to_string(), "Updated title".to_string())]
    );
    assert_eq!(
        notifier.worktrees.lock().unwrap().as_slice(),
        &["/repo/changed".to_string()]
    );
    assert_eq!(
        repository.sessions.lock().unwrap()[0]
            .session()
            .provider_session_title(),
        Some("Updated title")
    );
}

#[tokio::test]
async fn test_provider_session_title_ingestion_読み取り失敗で他sessionの処理を止めない() {
    let repository = Arc::new(RecordingRepository::new(vec![
        session("failed", "provider-failed", None),
        session("continued", "provider-continued", None),
    ]));
    let gateway = Arc::new(FixedTitleGateway::new([
        (
            "provider-failed",
            Err(ProviderSessionTitleGatewayError::Unavailable),
        ),
        ("provider-continued", Ok(Some("Available title"))),
    ]));
    let notifier = Arc::new(RecordingNotifier::default());
    let usecase =
        ProviderSessionTitleIngestionUsecase::new(repository.clone(), gateway, notifier.clone());

    usecase.ingest_due().await;

    assert_eq!(
        repository.saved_titles.lock().unwrap().as_slice(),
        &[("continued".to_string(), "Available title".to_string())]
    );
    assert_eq!(
        notifier.worktrees.lock().unwrap().as_slice(),
        &["/repo/continued".to_string()]
    );
}

#[tokio::test]
async fn test_provider_session_title_ingestion_タイトル事実から単独sessionの表示名を初回と更新後に投影する(
) {
    let session_id = "display-session";
    let provider_session_id = "provider-display-session";
    let repository = Arc::new(RecordingRepository::new(vec![session(
        session_id,
        provider_session_id,
        None,
    )]));
    let gateway = Arc::new(FixedTitleGateway::new([(
        provider_session_id,
        Ok(Some("Generated title")),
    )]));
    let notifier = Arc::new(RecordingNotifier::default());
    let usecase =
        ProviderSessionTitleIngestionUsecase::new(repository.clone(), gateway.clone(), notifier);
    let root = SessionExecutionTreeRootFacts::new(
        session_id,
        "workspace",
        format!("/repo/{session_id}"),
        ProviderKind::Claude,
    )
    .unwrap();
    let meta = root.meta.clone();
    let mut records = root
        .into_facts()
        .into_iter()
        .enumerate()
        .map(|(index, (meta, fact))| NodeFactRecord {
            meta,
            seq: i64::try_from(index + 1).unwrap(),
            timestamp_ms: i64::try_from(index + 1).unwrap() * 1000,
            fact,
        })
        .collect::<Vec<_>>();

    usecase.ingest_due().await;
    let first_title = repository.saved_titles.lock().unwrap()[0].1.clone();
    records.push(NodeFactRecord {
        meta: meta.clone(),
        seq: 3,
        timestamp_ms: 3000,
        fact: NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
            title: first_title,
        }),
    });
    assert_eq!(
        standalone_session_title(session_id, &records),
        "Generated title"
    );

    gateway.set_title(provider_session_id, Some("Updated title"));
    for _ in 0..15 {
        usecase.ingest_due().await;
    }
    let updated_title = repository.saved_titles.lock().unwrap()[1].1.clone();
    records.push(NodeFactRecord {
        meta,
        seq: 4,
        timestamp_ms: 4000,
        fact: NodeFact::ProviderSessionTitleObserved(ProviderSessionTitleObservedFact {
            title: updated_title,
        }),
    });
    assert_eq!(
        standalone_session_title(session_id, &records),
        "Updated title"
    );
}
