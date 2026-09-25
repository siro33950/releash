use super::*;
use crate::domain::workflow::entities::workflow_execution::{LeafKind, LeafStart};
use std::sync::Mutex;

async fn retry_failed_nodes(
    gateway: &FakeStartup,
    failed: Vec<FailedNodeStart>,
) -> Result<(), NodeStartupError> {
    let queue = crate::usecase::work_queue::WorkQueueUsecase::new(gateway.clock.clone());
    super::retry_failed_nodes_with_queue(gateway, failed, &queue).await
}

struct FakeStartup {
    clock: std::sync::Arc<crate::usecase::work_queue::ImmediateWorkQueueRuntime>,
    failures_left: Mutex<usize>,
    starts: Mutex<Vec<Vec<String>>>,
    restarts: Mutex<Vec<String>>,
    waits: Mutex<Vec<std::time::Duration>>,
    restart_error: Mutex<Option<WorkflowRuntimeError>>,
    start_error: Mutex<Option<WorkflowRuntimeError>>,
    cancelled: bool,
    wait_cancelled: bool,
    pending_restart: bool,
    pending_start: bool,
}

impl FakeStartup {
    fn new(failures: usize) -> Self {
        Self {
            clock: Default::default(),
            failures_left: Mutex::new(failures),
            starts: Mutex::new(Vec::new()),
            restarts: Mutex::new(Vec::new()),
            waits: Mutex::new(Vec::new()),
            restart_error: Mutex::new(None),
            start_error: Mutex::new(None),
            cancelled: false,
            wait_cancelled: false,
            pending_restart: false,
            pending_start: false,
        }
    }
}

fn leaf(id: &str, kind: LeafKind) -> NodeStart {
    NodeStart::Leaf(LeafStart {
        node_execution_id: id.into(),
        node_name: "work".into(),
        kind,
        bindings: Vec::new(),
        item: None,
    })
}

#[async_trait::async_trait]
impl NodeStartupGateway for FakeStartup {
    async fn start(
        &self,
        starts: Vec<NodeStart>,
    ) -> Result<Vec<FailedNodeStart>, WorkflowRuntimeError> {
        if self.pending_start && starts[0].node_execution_id().starts_with("blocked") {
            return std::future::pending().await;
        }
        if let Some(error) = self.start_error.lock().unwrap().take() {
            return Err(error);
        }
        let ids: Vec<_> = starts
            .iter()
            .map(|start| start.node_execution_id().to_string())
            .collect();
        self.starts.lock().unwrap().push(ids.clone());
        let mut remaining = self.failures_left.lock().unwrap();
        if *remaining == 0 {
            return Ok(Vec::new());
        }
        *remaining -= 1;
        Ok(vec![FailedNodeStart {
            id: ids[0].clone(),
            kind: FailureKind::RestartRequired,
        }])
    }

    async fn restart(
        &self,
        id: &str,
        action: RetryAction,
    ) -> Result<Option<NodeStart>, WorkflowRuntimeError> {
        self.restarts.lock().unwrap().push(id.into());
        if id == "blocked" {
            if self.pending_restart {
                return std::future::pending().await;
            }
            if self.pending_start {
                tokio::time::sleep(std::time::Duration::from_secs(7)).await;
            }
        }
        if let Some(error) = self.restart_error.lock().unwrap().take() {
            return Err(error);
        }
        Ok((!self.cancelled).then(|| {
            leaf(
                &if action == RetryAction::Retry {
                    id.to_string()
                } else {
                    format!("{id}-next")
                },
                LeafKind::Session,
            )
        }))
    }

    async fn cancelled(&self) {
        if !self.wait_cancelled {
            std::future::pending::<()>().await;
        }
    }

    async fn wait(&self, duration: std::time::Duration) -> bool {
        use crate::usecase::work_queue::WorkQueueRuntime;
        assert_eq!(duration, std::time::Duration::ZERO);
        self.waits.lock().unwrap().push(self.clock.now());
        !self.wait_cancelled
    }
}

async fn start_nodes(
    gateway: &FakeStartup,
    starts: Vec<NodeStart>,
) -> Result<(), WorkflowRuntimeError> {
    let failed = gateway.start(starts).await?;
    retry_failed_nodes(gateway, failed)
        .await
        .map_err(|failure| failure.error)
}

#[tokio::test]
async fn test_起動再試行_4回を超えて成功まで継続する() {
    for kind in [LeafKind::Session, LeafKind::Command] {
        let gateway = FakeStartup::new(5);
        start_nodes(&gateway, vec![leaf("first", kind)])
            .await
            .unwrap();
        let starts = gateway.starts.lock().unwrap();
        assert_eq!(starts.len(), 6);
        assert_eq!(gateway.restarts.lock().unwrap().len(), 5);
        assert!(starts.windows(2).all(|pair| pair[0] != pair[1]));
        let waits = gateway.waits.lock().unwrap();
        assert_eq!(waits.len(), 5);
        let mut previous = std::time::Duration::ZERO;
        for (index, instant) in waits.iter().enumerate() {
            let delay = *instant - previous;
            previous = *instant;
            let base = RetryBackoff::CONFLICT.delay(index as u64 + 1, 1.0);
            assert!(delay >= base.mul_f64(0.8) && delay <= base.mul_f64(1.2));
        }
    }
}

#[tokio::test]
async fn startup_stops_retrying_after_success_and_leaves_successful_siblings_alone() {
    let gateway = FakeStartup::new(1);
    start_nodes(
        &gateway,
        vec![
            leaf("failed", LeafKind::Session),
            leaf("live", LeafKind::Command),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        *gateway.starts.lock().unwrap(),
        vec![vec!["failed", "live"], vec!["failed-next"]]
    );
    assert_eq!(*gateway.restarts.lock().unwrap(), vec!["failed"]);
}

#[tokio::test]
async fn startup_does_not_launch_an_attempt_cancelled_during_backoff() {
    let mut gateway = FakeStartup::new(usize::MAX);
    gateway.cancelled = true;
    start_nodes(&gateway, vec![leaf("first", LeafKind::Session)])
        .await
        .unwrap();
    assert_eq!(gateway.starts.lock().unwrap().len(), 1);
    assert_eq!(gateway.restarts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_自動再試行_待機の取消後は新attemptを作らない() {
    let mut gateway = FakeStartup::new(usize::MAX);
    gateway.wait_cancelled = true;
    retry_failed_nodes(&gateway, vec!["first".into()])
        .await
        .unwrap();
    assert!(gateway.restarts.lock().unwrap().is_empty());
    assert!(gateway.starts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_自動再試行_restartエラーを分類し後続nodeも処理する() {
    // Given
    for retryable in [true, false] {
        let gateway = FakeStartup::new(5);
        *gateway.restart_error.lock().unwrap() = Some(if retryable {
            WorkflowRuntimeError::Conflict("advanced".into())
        } else {
            WorkflowRuntimeError::SessionStore("broken".into())
        });
        // When
        let result = retry_failed_nodes(&gateway, vec!["first".into(), "second".into()]).await;
        // Then
        if retryable {
            result.unwrap();
        } else {
            assert_eq!(
                result.unwrap_err().node_execution_id.as_deref(),
                Some("first")
            );
        }
        assert!(gateway.starts.lock().unwrap().len() > 4);
        assert_eq!(gateway.starts.lock().unwrap()[0], ["second-next"]);
    }
}

#[tokio::test]
async fn test_自動再試行_後続nodeがなくても競合を再試行する() {
    // Given
    let gateway = FakeStartup::new(0);
    *gateway.restart_error.lock().unwrap() =
        Some(WorkflowRuntimeError::Conflict("advanced".into()));
    // When
    retry_failed_nodes(&gateway, vec!["first".into()])
        .await
        .unwrap();
    // Then
    assert_eq!(gateway.restarts.lock().unwrap().len(), 2);
    assert_eq!(gateway.starts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_自動再試行_startエラーにrestartの発生元を割り当てない() {
    // Given
    let gateway = FakeStartup::new(0);
    *gateway.restart_error.lock().unwrap() =
        Some(WorkflowRuntimeError::InvalidState("restart failed".into()));
    *gateway.start_error.lock().unwrap() =
        Some(WorkflowRuntimeError::SessionStore("start failed".into()));
    // When
    let failure = retry_failed_nodes(&gateway, vec!["first".into(), "second".into()])
        .await
        .unwrap_err();
    // Then
    assert!(failure.node_execution_id.is_none());
    assert!(
        matches!(failure.error, WorkflowRuntimeError::SessionStore(reason) if reason == "start failed")
    );
}

#[tokio::test]
async fn test_自動再試行_一時失敗は同じattemptを起動する() {
    // Given
    let gateway = FakeStartup::new(0);
    // When
    retry_failed_nodes(
        &gateway,
        vec![FailedNodeStart {
            id: "first".into(),
            kind: FailureKind::Temporary,
        }],
    )
    .await
    .unwrap();
    // Then
    assert_eq!(gateway.starts.lock().unwrap()[0], ["first"]);
}

#[tokio::test(start_paused = true)]
async fn test_node起動再試行_restartとstartに共通の20秒期限を適用し対象を解放する() {
    // Given
    for pending_restart in [true, false] {
        let mut gateway = FakeStartup::new(0);
        gateway.pending_restart = pending_restart;
        gateway.pending_start = !pending_restart;
        let queue = crate::usecase::work_queue::work_queue_tests::queue();
        let started = tokio::time::Instant::now();
        // When
        let failure = tokio::time::timeout(
            std::time::Duration::from_secs(21),
            super::retry_failed_nodes_with_queue(
                &gateway,
                vec!["blocked".into(), "other".into()],
                &queue,
            ),
        )
        .await
        .expect("node attempt must end at its 20 second deadline")
        .unwrap_err();
        // Then
        assert_eq!(failure.error.failure_kind(), FailureKind::Expired);
        let target = if pending_restart {
            "blocked"
        } else {
            "blocked-next"
        };
        assert_eq!(failure.node_execution_id.as_deref(), Some(target));
        assert_eq!(started.elapsed(), std::time::Duration::from_millis(20_010));
        assert_eq!(*gateway.restarts.lock().unwrap(), ["blocked", "other"]);
        assert_eq!(*gateway.starts.lock().unwrap(), [vec!["other-next"]]);
        let records = queue.records(target).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record.kind, FailureKind::Expired);
        assert!(records[0].requires_attention);
        assert_eq!(
            queue
                .execute(
                    crate::usecase::work_queue::WorkKey::new("workflow_node_start", "blocked"),
                    RetryBackoff::ITEM,
                    |_| async { Ok(42) },
                )
                .await
                .unwrap(),
            42
        );
    }
}
