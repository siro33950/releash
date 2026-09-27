use super::*;

#[derive(Default)]
struct RecordingOutput {
    calls: Mutex<Vec<(String, String, Option<(String, u64)>)>>,
}

impl StateSubscriptionOutput for RecordingOutput {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn subscribe_changes(&self) -> tokio::sync::broadcast::Receiver<StateChangeSource> {
        tokio::sync::broadcast::channel(1).1
    }

    fn invalidate(&self, _: StateChangeSource) {
        unreachable!()
    }

    fn publish(
        &self,
        _: &str,
        _: StateValue,
        _: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        unreachable!()
    }

    fn open(&self, _: String) -> Result<(), SubscriptionError> {
        unreachable!()
    }

    fn close(&self, _: &str) {
        unreachable!()
    }

    fn start(
        &self,
        client: &str,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError> {
        self.calls.lock().push((
            client.into(),
            target.into(),
            cursor.map(|(epoch, sequence)| (epoch.into(), sequence)),
        ));
        Ok(())
    }

    fn start_with_snapshot(
        &self,
        _: &str,
        _: &str,
        _: StateValue,
        _: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError> {
        unreachable!()
    }

    fn stop(&self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        self.calls.lock().push((client.into(), target.into(), None));
        Ok(())
    }

    fn active_targets(&self) -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }

    fn ensure_active(&self, _: &str) -> Result<(), SubscriptionError> {
        unreachable!()
    }

    fn release_inactive_snapshots(&self) {}

    fn needs_snapshot(&self, _: &str, _: Option<(&str, u64)>) -> Result<bool, SubscriptionError> {
        unreachable!()
    }

    fn terminal_pending_amount(&self, _: &str, _: &str) -> usize {
        unreachable!()
    }

    fn set_terminal_snapshot(
        &self,
        _: &str,
        _: u64,
        _: u64,
        _: StateValue,
    ) -> Result<(), SubscriptionError> {
        unreachable!()
    }

    fn terminal_reset_clients(&self, _: &str) -> Vec<String> {
        unreachable!()
    }

    fn terminal_report_units(&self) -> usize {
        unreachable!()
    }

    fn set_terminal_input(&self, _: &str, _: &str, _: &str) {
        unreachable!()
    }

    fn remove_terminal_input(&self, _: &str, _: &str) -> Option<String> {
        None
    }

    fn has_terminal_input(&self, _: &str, _: &str) -> bool {
        unreachable!()
    }

    fn terminal_targets(&self, _: &str) -> Vec<String> {
        unreachable!()
    }

    fn terminal_state_sink(
        &self,
    ) -> Arc<dyn crate::usecase::terminal_surface::output::TerminalSurfaceStateSink> {
        unreachable!()
    }
}

struct PendingTimer;

impl SubscriptionTimer for PendingTimer {
    fn interval(&self, _: std::time::Duration) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>> {
        Box::pin(futures_util::stream::pending())
    }
}

#[test]
fn test_購読手順_対象を検証して開始停止をoutput_boundaryへ渡す() {
    let output = Arc::new(RecordingOutput::default());
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer));

    assert_eq!(
        usecase.start("client", "missing", None),
        Err(SubscriptionError::UnknownTarget)
    );
    usecase
        .start("client", REPO_PATHS, Some(("epoch", 4)))
        .unwrap();
    usecase.stop("client", REPO_PATHS).unwrap();

    assert_eq!(
        *output.calls.lock(),
        vec![
            (
                "client".into(),
                REPO_PATHS.into(),
                Some(("epoch".into(), 4))
            ),
            ("client".into(), REPO_PATHS.into(), None),
        ]
    );
}
