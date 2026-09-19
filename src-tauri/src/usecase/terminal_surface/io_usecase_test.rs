use super::*;
use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceInputIngressError, TerminalSurfaceInputIngressRegistry,
    TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError,
};
use crate::domain::terminal_surface::gateway::{
    TerminalRuntimeSpawnRequest, TerminalSurfaceGatewayError, TerminalSurfaceInputUnavailableCause,
    TerminalSurfaceRepository,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use parking_lot::Mutex;

pub(crate) struct FakePtyGateway {
    pub(crate) resizes: Mutex<Vec<(String, u16, u16)>>,
    pub(crate) writes: Mutex<Vec<(String, String)>>,
    input_ingress: Mutex<TerminalSurfaceInputIngressRegistry>,
    pub(crate) surface: Option<TerminalSurface>,
    pub(crate) snapshot_gate:
        Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    pub(crate) deactivated: Mutex<Vec<String>>,
}

impl FakePtyGateway {
    pub(crate) fn new() -> Self {
        Self {
            resizes: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            input_ingress: Mutex::new(TerminalSurfaceInputIngressRegistry::default()),
            surface: None,
            snapshot_gate: Mutex::new(None),
            deactivated: Mutex::new(Vec::new()),
        }
    }
}

impl TerminalSurfaceRepository for FakePtyGateway {
    fn find_summary_by_session_key(
        &self,
        _session_key: &str,
    ) -> Option<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        self.surface.as_ref().map(TerminalSurface::summary)
    }

    fn list_summaries(
        &self,
    ) -> Vec<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        Vec::new()
    }
}

impl TerminalSurfaceGateway for FakePtyGateway {
    fn next_runtime_generation(&self) -> u64 {
        1
    }

    fn spawn_runtime(
        &self,
        _request: TerminalRuntimeSpawnRequest,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn insert_surface(&self, _surface: TerminalSurface) {}

    fn start_output_reader(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn snapshot(&self, _runtime_generation: u64) -> Option<TerminalSurface> {
        let gate = self.snapshot_gate.lock().take();
        if let Some((started, release)) = gate {
            started.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        self.surface.clone()
    }

    fn select_kill_targets_by_worktree(&self, _worktree_path: &str) -> Vec<u64> {
        Vec::new()
    }

    fn remove_surface(&self, _runtime_generation: u64) -> Option<TerminalSurface> {
        None
    }

    fn reserve_spawn_slot(
        &self,
        session_key: &str,
    ) -> Result<TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError> {
        Ok(TerminalSurfaceSpawnReservation {
            session_key: session_key.to_string(),
        })
    }

    fn complete_spawn_slot(&self, _reservation: &TerminalSurfaceSpawnReservation) {}

    fn rollback_spawn_slot(&self, _reservation: &TerminalSurfaceSpawnReservation) {}

    fn activate_input_attachment(&self, session_key: &str, attachment_id: &str) {
        self.input_ingress
            .lock()
            .activate(session_key, attachment_id);
    }

    fn deactivate_input_attachment(&self, session_key: &str, attachment_id: &str) {
        self.input_ingress
            .lock()
            .deactivate(session_key, attachment_id);
        self.deactivated.lock().push(attachment_id.into());
    }

    fn write_attached(
        &self,
        session_key: &str,
        attachment_id: &str,
        sequence: u64,
        data: &str,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let mut ingress = self.input_ingress.lock();
        let ready = ingress
            .admit(session_key, attachment_id, sequence, data.to_string())
            .map_err(|error| {
                let cause = match error {
                    TerminalSurfaceInputIngressError::StaleAttachment => {
                        TerminalSurfaceInputUnavailableCause::StaleAttachment
                    }
                    TerminalSurfaceInputIngressError::PendingCapacityExceeded => {
                        TerminalSurfaceInputUnavailableCause::PendingCapacityExceeded
                    }
                };
                TerminalSurfaceGatewayError::new(cause.internal_cause())
            })?;
        for input in ready {
            self.write(session_key, &input.data)?;
        }
        Ok(())
    }

    fn write(&self, session_key: &str, data: &str) -> Result<(), TerminalSurfaceGatewayError> {
        self.writes
            .lock()
            .push((session_key.to_string(), data.to_string()));
        Ok(())
    }

    fn resize(
        &self,
        session_key: &str,
        rows: u16,
        cols: u16,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        self.resizes.lock().push((session_key.into(), rows, cols));
        Ok(())
    }

    fn request_runtime_stop(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn remove_runtime(&self, _runtime_generation: u64) {}
}

#[test]
fn test_ターミナル画面_パス入力_引用符処理して結合後に書き込む() {
    let gateway = FakePtyGateway::new();
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();

    write_paths(
        &gateway,
        &owner,
        &[
            "/tmp/a.txt".to_string(),
            "/tmp/my file.txt".to_string(),
            "/tmp/it's.txt".to_string(),
        ],
    )
    .unwrap();

    assert_eq!(
        *gateway.writes.lock(),
        vec![(
            owner.stable_key(),
            "/tmp/a.txt '/tmp/my file.txt' '/tmp/it'\\''s.txt'".to_string()
        )]
    );
}

#[test]
fn test_ターミナル画面_パス入力_空配列では何もしない() {
    let gateway = FakePtyGateway::new();
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();

    write_paths(&gateway, &owner, &[]).unwrap();

    assert!(gateway.writes.lock().is_empty());
}
