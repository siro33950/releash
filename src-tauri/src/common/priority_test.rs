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
async fn test_優先度包み_対象外は通し拒否と受理を通知する() {
    // Given
    let limits = Arc::new(PriorityLimits::new(1, &[("default", 1)], 0));
    let held = limits.admit("default", None).await.unwrap();
    let events = Arc::new(RecordingEvents::default());
    let gate = PriorityGate::new(
        limits,
        |path| (path != "/bypass").then_some("default"),
        events.clone(),
    );
    let calls = Cell::new(0);

    // When / Then
    assert_eq!(
        gate.run(
            "/bypass",
            None,
            || async {
                calls.set(calls.get() + 1);
                Ok::<_, &'static str>(())
            },
            |_| "rejected",
        )
        .await,
        Ok(())
    );
    assert_eq!(calls.get(), 1);
    assert!(events.0.lock().unwrap().is_empty());

    assert_eq!(
        gate.run(
            "/limited",
            None,
            || async {
                calls.set(calls.get() + 1);
                Ok::<_, &'static str>(())
            },
            |_| "rejected",
        )
        .await,
        Err("rejected")
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(
        *events.0.lock().unwrap(),
        ["/limited: default requests rejected: queue_full"]
    );

    drop(held);
    assert_eq!(
        gate.run(
            "/limited",
            None,
            || async {
                calls.set(calls.get() + 1);
                Ok::<_, &'static str>(())
            },
            |_| "rejected",
        )
        .await,
        Ok(())
    );
    assert_eq!(calls.get(), 2);
    assert_eq!(
        *events.0.lock().unwrap(),
        [
            "/limited: default requests rejected: queue_full",
            "admitted"
        ]
    );
}
