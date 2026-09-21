use super::*;
use crate::domain::workflow::entities::workflow_execution::{LeafKind, LeafStart};
use std::sync::Mutex;

struct FakeStartup {
    failures_left: Mutex<usize>,
    starts: Mutex<Vec<Vec<String>>>,
    restarts: Mutex<Vec<String>>,
    waits: Mutex<Vec<std::time::Duration>>,
    cancelled: bool,
    wait_cancelled: bool,
}

impl FakeStartup {
    fn new(failures: usize) -> Self {
        Self {
            failures_left: Mutex::new(failures),
            starts: Mutex::new(Vec::new()),
            restarts: Mutex::new(Vec::new()),
            waits: Mutex::new(Vec::new()),
            cancelled: false,
            wait_cancelled: false,
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
    async fn start(&self, starts: Vec<NodeStart>) -> Result<Vec<String>, WorkflowRuntimeError> {
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
        Ok(vec![ids[0].clone()])
    }

    async fn restart(&self, id: &str) -> Result<Option<NodeStart>, WorkflowRuntimeError> {
        self.restarts.lock().unwrap().push(id.into());
        Ok((!self.cancelled).then(|| leaf(&format!("{id}-next"), LeafKind::Session)))
    }

    async fn wait(&self, duration: std::time::Duration) -> bool {
        self.waits.lock().unwrap().push(duration);
        !self.wait_cancelled
    }
}

async fn start_nodes(
    gateway: &FakeStartup,
    starts: Vec<NodeStart>,
) -> Result<(), WorkflowRuntimeError> {
    let failed = gateway.start(starts).await?;
    retry_failed_nodes(gateway, failed).await
}

#[tokio::test]
async fn startup_retries_four_times_with_new_attempts_and_increasing_delays() {
    for kind in [LeafKind::Session, LeafKind::Command] {
        let gateway = FakeStartup::new(usize::MAX);
        start_nodes(&gateway, vec![leaf("first", kind)])
            .await
            .unwrap();
        let starts = gateway.starts.lock().unwrap();
        assert_eq!(starts.len(), 5);
        assert_eq!(gateway.restarts.lock().unwrap().len(), 4);
        assert!(starts.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(
            *gateway.waits.lock().unwrap(),
            [1, 2, 4, 8].map(std::time::Duration::from_secs)
        );
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
