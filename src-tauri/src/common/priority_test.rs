use super::*;
use std::cell::Cell;
use std::sync::Mutex;

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<String>>);

impl PriorityEvents for RecordingEvents {
    fn rejected(&self, path: &str, rejection: &Rejection) {
        self.0.lock().unwrap().push(format!("{path}: {rejection}"));
    }

    fn admitted(&self) {
        self.0.lock().unwrap().push("admitted".into());
    }
}

#[tokio::test]
async fn test_優先度包み_対象外は枠を取らず通知しない() {
    // Given
    let limits = Arc::new(PriorityLimits::new(1, &[("default", 1)], 0));
    let _held = limits.admit("default", None).await.unwrap();
    let events = Arc::new(RecordingEvents::default());
    let gate = PriorityGate::new(
        limits,
        |path| (path != "/bypass").then_some("default"),
        events.clone(),
    );
    let calls = Cell::new(0);

    // When
    assert_eq!(
        gate.run(
            "/bypass".to_owned(),
            String::as_str,
            None,
            |_| async {
                calls.set(calls.get() + 1);
                Ok::<_, &'static str>(())
            },
            |_| "rejected",
        )
        .await,
        Ok(())
    );
    // Then
    assert_eq!(calls.get(), 1);
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_優先度包み_満席で拒否を通知し次を呼ばない() {
    // Given
    let limits = Arc::new(PriorityLimits::new(1, &[("default", 1)], 0));
    let _held = limits.admit("default", None).await.unwrap();
    let events = Arc::new(RecordingEvents::default());
    let gate = PriorityGate::new(limits, |_| Some("default"), events.clone());
    let calls = Cell::new(0);

    // When
    assert_eq!(
        gate.run(
            "/limited".to_owned(),
            String::as_str,
            None,
            |_| async {
                calls.set(calls.get() + 1);
                Ok::<_, &'static str>(())
            },
            |_| "rejected",
        )
        .await,
        Err("rejected")
    );
    // Then
    assert_eq!(calls.get(), 0);
    assert_eq!(
        *events.0.lock().unwrap(),
        ["/limited: default requests rejected: queue_full"]
    );
}

#[tokio::test]
async fn test_優先度包み_受理を通知してから次を呼ぶ() {
    // Given
    let limits = Arc::new(PriorityLimits::new(1, &[("default", 1)], 0));
    let events = Arc::new(RecordingEvents::default());
    let gate = PriorityGate::new(limits, |_| Some("default"), events.clone());

    // When
    assert_eq!(
        gate.run(
            "/limited".to_owned(),
            String::as_str,
            None,
            |_| async {
                events.0.lock().unwrap().push("next".into());
                Ok::<_, &'static str>(())
            },
            |_| "rejected",
        )
        .await,
        Ok(())
    );
    // Then
    assert_eq!(*events.0.lock().unwrap(), ["admitted", "next"]);
}

#[tokio::test]
async fn test_入口の分類を変えても同じ同時実行枠を共有する() {
    let limits = Arc::new(PriorityLimits::new(1, &[("default", 1)], 0));
    let gate = PriorityGate::new(
        limits.clone(),
        |_| Some("default"),
        Arc::new(RecordingEvents::default()),
    );
    let local = gate.with_classifier(|_| Some("default"));
    assert!(std::ptr::eq(gate.limits(), local.limits()));
    let _seat = limits.admit("default", None).await.unwrap();
    let result = local
        .run(
            "/local",
            |path| path,
            None,
            |_| async { Ok::<_, Rejection>(()) },
            |error| error,
        )
        .await;
    assert!(result.is_err());
}
