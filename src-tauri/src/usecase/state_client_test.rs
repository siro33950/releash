use super::*;
use crate::domain::state_subscription::connection::ReceivedState;
use crate::domain::state_subscription::Version;
use parking_lot::Mutex;
use std::collections::VecDeque;

type Starts = Arc<Mutex<Vec<(usize, String, Option<Version>)>>>;
struct Gateway {
    connects: Mutex<usize>,
    starts: Starts,
    stops: Arc<Mutex<Vec<(usize, String)>>>,
    streams: Mutex<VecDeque<tokio::sync::mpsc::Receiver<ReceivedState>>>,
}
struct Connection {
    id: usize,
    starts: Starts,
    stops: Arc<Mutex<Vec<(usize, String)>>>,
    stream: tokio::sync::mpsc::Receiver<ReceivedState>,
}
#[async_trait::async_trait]
impl StateConnection for Connection {
    async fn start(
        &mut self,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), StateClientError> {
        self.starts
            .lock()
            .push((self.id, target.into(), version.cloned()));
        Ok(())
    }
    async fn stop(&mut self, target: &str) -> Result<(), StateClientError> {
        self.stops.lock().push((self.id, target.into()));
        Ok(())
    }
    async fn receive(&mut self) -> Result<ReceivedState, StateClientError> {
        self.stream
            .recv()
            .await
            .ok_or_else(|| StateClientError("closed".into()))
    }
}
#[async_trait::async_trait]
impl StateClientGateway for Gateway {
    async fn connect(&self) -> Result<Box<dyn StateConnection>, StateClientError> {
        let mut connects = self.connects.lock();
        *connects += 1;
        let stream = self
            .streams
            .lock()
            .pop_front()
            .ok_or_else(|| StateClientError("no stream".into()))?;
        Ok(Box::new(Connection {
            id: *connects,
            starts: self.starts.clone(),
            stops: self.stops.clone(),
            stream,
        }))
    }
}
fn gateway(streams: Vec<tokio::sync::mpsc::Receiver<ReceivedState>>) -> Arc<Gateway> {
    Arc::new(Gateway {
        connects: Default::default(),
        starts: Default::default(),
        stops: Default::default(),
        streams: Mutex::new(streams.into()),
    })
}
fn event(target: &str, sequence: u64, snapshot: bool) -> ReceivedState {
    ReceivedState {
        target: target.into(),
        version: Version {
            epoch: "daemon".into(),
            sequence,
        },
        snapshot,
        value: Some(StateValue::RepositoryPaths(vec![format!(
            "/{target}/{sequence}"
        )])),
    }
}
fn receiver(values: &Arc<Mutex<Vec<StateValue>>>) -> StateReceiver {
    let values = values.clone();
    Arc::new(move |value| {
        values.lock().push(value);
        Ok(())
    })
}
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn test_受信_無通信後に最後の版で再開し停止後は再接続しない() {
    // Given
    let (sender, stream) = tokio::sync::mpsc::channel(4);
    let (_next_sender, next_stream) = tokio::sync::mpsc::channel(4);
    let gateway = gateway(vec![stream, next_stream]);
    let usecase = StateClientUsecase::new(
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let received = Arc::new(Mutex::new(Vec::new()));
    usecase.start("view".into(), "a".into(), receiver(&received));
    // When
    sender.send(event("a", 7, true)).await.unwrap();
    settle().await;
    tokio::time::advance(Duration::from_secs(31)).await;
    settle().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    // Then
    assert_eq!(
        *received.lock(),
        [StateValue::RepositoryPaths(vec!["/a/7".into()])]
    );
    assert_eq!(
        *gateway.starts.lock(),
        [
            (1, "a".into(), None),
            (
                2,
                "a".into(),
                Some(Version {
                    epoch: "daemon".into(),
                    sequence: 7
                })
            ),
        ]
    );
    usecase.stop("old-view");
    settle().await;
    assert!(gateway.stops.lock().is_empty());
    usecase.stop("view");
    settle().await;
    assert_eq!(*gateway.stops.lock(), [(2, "a".into())]);
    tokio::time::advance(Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(*gateway.connects.lock(), 2);
}

#[tokio::test(start_paused = true)]
async fn test_受信_複数対象を同じ接続で受信し一方の停止後も他方を継続する() {
    // Given
    let (sender, stream) = tokio::sync::mpsc::channel(8);
    let gateway = gateway(vec![stream]);
    let usecase = StateClientUsecase::new(
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let a = Arc::new(Mutex::new(Vec::new()));
    let b = Arc::new(Mutex::new(Vec::new()));
    usecase.start("view-a".into(), "a".into(), receiver(&a));
    settle().await;
    // When
    usecase.start("view-b".into(), "b".into(), receiver(&b));
    settle().await;
    sender.send(event("a", 7, true)).await.unwrap();
    sender.send(event("b", 2, true)).await.unwrap();
    settle().await;
    sender.send(event("a", 8, false)).await.unwrap();
    sender.send(event("b", 3, false)).await.unwrap();
    settle().await;
    // Then
    assert_eq!(*gateway.connects.lock(), 1);
    assert_eq!(
        *gateway.starts.lock(),
        [(1, "a".into(), None), (1, "b".into(), None)]
    );
    assert_eq!(a.lock().len(), 2);
    assert_eq!(b.lock().len(), 2);
    usecase.stop("view-a");
    settle().await;
    sender.send(event("a", 9, false)).await.unwrap();
    sender.send(event("b", 4, false)).await.unwrap();
    settle().await;
    assert_eq!(*gateway.stops.lock(), [(1, "a".into())]);
    assert_eq!(a.lock().len(), 2);
    assert_eq!(
        b.lock().last(),
        Some(&StateValue::RepositoryPaths(vec!["/b/4".into()]))
    );
    assert_eq!(*gateway.connects.lock(), 1);
}

#[tokio::test(start_paused = true)]
async fn test_受信_再接続で全対象を各版から再開し同じ対象の受信者追加も他対象を止めない() {
    // Given
    let (sender, stream) = tokio::sync::mpsc::channel(8);
    let (next_sender, next_stream) = tokio::sync::mpsc::channel(8);
    let gateway = gateway(vec![stream, next_stream]);
    let usecase = StateClientUsecase::new(
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let a = Arc::new(Mutex::new(Vec::new()));
    let b = Arc::new(Mutex::new(Vec::new()));
    usecase.start("view-a".into(), "a".into(), receiver(&a));
    usecase.start("view-b".into(), "b".into(), receiver(&b));
    settle().await;
    sender.send(event("a", 7, true)).await.unwrap();
    sender.send(event("b", 2, true)).await.unwrap();
    settle().await;
    // When
    drop(sender);
    settle().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    // Then
    assert_eq!(*gateway.connects.lock(), 2);
    for (target, sequence) in [("a", 7), ("b", 2)] {
        assert!(gateway.starts.lock().contains(&(
            2,
            target.into(),
            Some(Version {
                epoch: "daemon".into(),
                sequence
            })
        )));
    }
    let new_a = Arc::new(Mutex::new(Vec::new()));
    usecase.start("new-a".into(), "a".into(), receiver(&new_a));
    settle().await;
    usecase.stop("view-a");
    settle().await;
    assert!(gateway.stops.lock().is_empty());
    assert_eq!(gateway.starts.lock().len(), 4);
    assert_eq!(new_a.lock().len(), 1);
    next_sender.send(event("a", 8, true)).await.unwrap();
    next_sender.send(event("b", 3, false)).await.unwrap();
    settle().await;
    assert_eq!(*gateway.connects.lock(), 2);
    assert_eq!(a.lock().len(), 1);
    assert_eq!(new_a.lock().len(), 2);
    assert_eq!(b.lock().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn test_受信_同じ対象の二つの受信者に配り最後の停止だけを送る() {
    // Given
    let (sender, stream) = tokio::sync::mpsc::channel(8);
    let gateway = gateway(vec![stream]);
    let usecase = StateClientUsecase::new(
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let first = Arc::new(Mutex::new(Vec::new()));
    let second = Arc::new(Mutex::new(Vec::new()));
    usecase.start("first".into(), "a".into(), receiver(&first));
    sender.send(event("a", 1, true)).await.unwrap();
    settle().await;
    // When
    usecase.start("second".into(), "a".into(), receiver(&second));
    settle().await;
    sender.send(event("a", 2, false)).await.unwrap();
    settle().await;
    // Then
    assert_eq!(*first.lock(), *second.lock());
    assert_eq!(first.lock().len(), 2);
    assert_eq!(*gateway.starts.lock(), [(1, "a".into(), None)]);
    usecase.stop("first");
    settle().await;
    assert!(gateway.stops.lock().is_empty());
    sender.send(event("a", 3, false)).await.unwrap();
    settle().await;
    assert_eq!(first.lock().len(), 2);
    assert_eq!(second.lock().len(), 3);
    usecase.stop("second");
    settle().await;
    assert_eq!(*gateway.stops.lock(), [(1, "a".into())]);
}
