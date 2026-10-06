use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceInputIngressError, TerminalSurfaceInputIngressRegistry,
    TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError,
};
use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
use crate::domain::terminal_surface::gateway::{
    TerminalRuntimeSpawnRequest, TerminalSurfaceGatewayError, TerminalSurfaceInputUnavailableCause,
    TerminalSurfaceRepository,
};
use parking_lot::Mutex;

type ShutdownGate = (
    &'static str,
    tokio::sync::oneshot::Sender<()>,
    std::sync::mpsc::Receiver<()>,
);

pub struct FakePtyGateway {
    pub list_summaries_calls: Mutex<usize>,
    pub resizes: Mutex<Vec<(String, u16, u16)>>,
    pub(crate) writes: Mutex<Vec<(String, String)>>,
    input_ingress: Mutex<TerminalSurfaceInputIngressRegistry>,
    pub(crate) surface: Option<TerminalSurface>,
    pub(crate) snapshot_unavailable: Mutex<bool>,
    pub additional_surfaces: Vec<TerminalSurface>,
    pub(crate) snapshot_gate:
        Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    pub(crate) deactivated: Mutex<Vec<String>>,
    pub shutdown_surfaces: Vec<TerminalSurface>,
    pub(crate) shutdown_failures: Vec<(&'static str, u64)>,
    pub shutdown_gate: Mutex<Option<ShutdownGate>>,
    pub(crate) shutdown_failure_id: String,
    pub(crate) shutdown_calls: Mutex<Vec<(&'static str, u64)>>,
}

impl FakePtyGateway {
    pub fn new() -> Self {
        Self {
            list_summaries_calls: Mutex::new(0),
            resizes: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            input_ingress: Mutex::new(TerminalSurfaceInputIngressRegistry::default()),
            surface: None,
            snapshot_unavailable: Mutex::new(false),
            additional_surfaces: Vec::new(),
            snapshot_gate: Mutex::new(None),
            deactivated: Mutex::new(Vec::new()),
            shutdown_surfaces: Vec::new(),
            shutdown_failures: Vec::new(),
            shutdown_gate: Mutex::new(None),
            shutdown_failure_id: uuid::Uuid::new_v4().to_string(),
            shutdown_calls: Mutex::new(Vec::new()),
        }
    }
}

impl FakePtyGateway {
    fn shutdown_step(
        &self,
        stage: &'static str,
        generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        self.shutdown_calls.lock().push((stage, generation));
        let mut gate = self.shutdown_gate.lock();
        if gate
            .as_ref()
            .is_some_and(|(blocked, _, _)| *blocked == stage)
        {
            let (_, started, release) = gate.take().unwrap();
            println!("terminal-blocked:{stage}");
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        drop(gate);
        if self.shutdown_failures.contains(&(stage, generation)) {
            Err(TerminalSurfaceGatewayError::NotFound(format!(
                "{stage} failed {}",
                self.shutdown_failure_id
            )))
        } else {
            Ok(())
        }
    }
}

impl TerminalSurfaceRepository for FakePtyGateway {
    fn find_summary_by_session_key(
        &self,
        session_key: &str,
    ) -> Option<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        self.surface
            .as_ref()
            .or_else(|| {
                self.additional_surfaces
                    .iter()
                    .find(|surface| surface.session_key == session_key)
            })
            .map(TerminalSurface::summary)
    }

    fn list_summaries(
        &self,
    ) -> Vec<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        *self.list_summaries_calls.lock() += 1;
        self.shutdown_surfaces
            .iter()
            .chain(self.additional_surfaces.iter())
            .map(TerminalSurface::summary)
            .collect()
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

    fn snapshot(&self, runtime_generation: u64) -> Option<TerminalSurface> {
        if *self.snapshot_unavailable.lock() {
            return None;
        }
        let gate = self.snapshot_gate.lock().take();
        if let Some((started, release)) = gate {
            started.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        self.surface.clone().or_else(|| {
            self.additional_surfaces
                .iter()
                .find(|surface| surface.runtime_generation.value() == runtime_generation)
                .cloned()
        })
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
                TerminalSurfaceGatewayError::InputUnavailable(cause)
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
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        self.shutdown_step("stop", runtime_generation)
    }

    fn wait_runtime_output_drain(
        &self,
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        self.shutdown_step("drain", runtime_generation)
    }

    fn flush_checkpoints(&self) -> Result<(), TerminalSurfaceGatewayError> {
        self.shutdown_step("flush", 0)
    }

    fn remove_runtime(&self, _runtime_generation: u64) {}
}

impl Default for FakePtyGateway {
    fn default() -> Self {
        Self::new()
    }
}
