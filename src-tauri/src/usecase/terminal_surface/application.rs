use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceMutationRejected, TerminalSurfaceRuntimeLifecycle,
    TerminalSurfaceSummary,
};
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEventSource, TerminalSurfaceGateway,
};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::usecase::terminal_surface::error::UsecaseError;
use crate::usecase::terminal_surface::spawn_usecase::GetOrSpawnTerminalOutcome;

#[derive(Clone)]
pub(crate) struct TerminalSurfaceApplication {
    performance: Arc<dyn crate::usecase::telemetry::PerformanceOutput>,
    gateway: Arc<dyn TerminalSurfaceGateway + Send + Sync>,
    event_source: Arc<dyn TerminalSurfaceEventSource>,
    runtime_lifecycle: Arc<RwLock<TerminalSurfaceRuntimeLifecycle>>,
    resize_tails: Arc<Mutex<HashMap<String, std::sync::mpsc::Receiver<()>>>>,
}

struct TerminalSurfaceResizeCompletion {
    application: TerminalSurfaceApplication,
    session_key: String,
    done: Option<std::sync::mpsc::Sender<()>>,
}

impl Drop for TerminalSurfaceResizeCompletion {
    fn drop(&mut self) {
        drop(self.done.take());
        let mut tails = self.application.resize_tails.lock().unwrap();
        if tails.get(&self.session_key).is_some_and(|tail| {
            matches!(
                tail.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Disconnected)
            )
        }) {
            tails.remove(&self.session_key);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OwnedTerminalSummaryLookup {
    Found(TerminalSurfaceSummary),
    Absent,
    OwnerMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TerminalSurfaceStreamItem {
    Snapshot(TerminalSurface),
    Output {
        session_key: String,
        data: Arc<str>,
        sequence: u64,
    },
    Resize {
        session_key: String,
        cols: u16,
        rows: u16,
        sequence: u64,
    },
    Exit {
        session_key: String,
        exit_code: Option<i32>,
        sequence: u64,
    },
}

impl TerminalSurfaceApplication {
    fn mutation_rejected(_: TerminalSurfaceMutationRejected) -> UsecaseError {
        UsecaseError::Gateway("Terminal Surface runtime is shutting down".to_string())
    }

    pub(crate) fn new(
        performance: Arc<dyn crate::usecase::telemetry::PerformanceOutput>,
        gateway: Arc<dyn TerminalSurfaceGateway + Send + Sync>,
        event_source: Arc<dyn TerminalSurfaceEventSource>,
    ) -> Self {
        Self {
            performance,
            gateway,
            event_source,
            resize_tails: Arc::new(Mutex::new(HashMap::new())),
            runtime_lifecycle: Arc::new(RwLock::new(TerminalSurfaceRuntimeLifecycle::new(
                "application-process".to_string(),
            ))),
        }
    }

    fn admit_mutation(
        &self,
    ) -> Result<RwLockReadGuard<'_, TerminalSurfaceRuntimeLifecycle>, UsecaseError> {
        let lifecycle = self
            .runtime_lifecycle
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        lifecycle
            .admit_mutation()
            .map_err(Self::mutation_rejected)?;
        Ok(lifecycle)
    }

    pub(crate) fn connect_state(
        &self,
        sink: Arc<dyn crate::domain::terminal_surface::gateway::TerminalSurfaceStateSink>,
    ) {
        self.event_source.set_state_sink(sink.clone());
        for summary in self.gateway.list_summaries() {
            sink.initialize(&summary);
        }
    }

    pub(crate) fn with_output_order(&self, runtime_generation: u64, visit: &mut dyn FnMut()) {
        self.gateway.with_output_order(runtime_generation, visit);
    }

    pub(crate) fn visit_snapshot(
        &self,
        owner: &TerminalSurfaceOwner,
        visit: &mut dyn FnMut(TerminalSurface),
    ) -> Result<(), UsecaseError> {
        let summary = self.owned_summary(owner)?;
        if self
            .gateway
            .visit_snapshot(summary.runtime_generation.value(), visit)
        {
            Ok(())
        } else {
            Err(UsecaseError::Gateway(
                "Terminal snapshot unavailable".into(),
            ))
        }
    }

    pub(crate) fn subscribe_output(
        &self,
        owner: &TerminalSurfaceOwner,
        client: &str,
        input_id: &str,
        units: usize,
    ) {
        self.event_source
            .subscribe_output(&owner.stable_key(), client, units);
        self.gateway
            .activate_input_attachment(&owner.stable_key(), input_id);
    }

    pub(crate) fn unsubscribe_output(
        &self,
        owner: &TerminalSurfaceOwner,
        client: &str,
        input_id: &str,
    ) {
        self.event_source
            .unsubscribe_output(&owner.stable_key(), client);
        self.gateway
            .deactivate_input_attachment(&owner.stable_key(), input_id);
    }

    pub(crate) fn reset_output(&self, owner: &TerminalSurfaceOwner, client: &str) {
        self.event_source
            .subscribe_output(&owner.stable_key(), client, 0);
    }

    pub(crate) fn processed_output(
        &self,
        owner: &TerminalSurfaceOwner,
        client: &str,
        units: usize,
    ) {
        self.event_source
            .processed_output(&owner.stable_key(), client, units);
    }

    pub(crate) fn summaries(&self) -> Vec<TerminalSurfaceSummary> {
        self.gateway.list_summaries()
    }

    pub(crate) fn subscribe_events(
        &self,
    ) -> crate::domain::terminal_surface::gateway::TerminalSurfaceEventStream {
        self.event_source.subscribe()
    }

    /// registryのsummaryだけで答える読み取り。scrollback全量のreplay再構築
    /// （emulator/registryロック保持のsnapshot materialization）を伴わない。
    pub(crate) fn find_owned_summary(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> OwnedTerminalSummaryLookup {
        let Some(summary) = self
            .gateway
            .find_summary_by_session_key(&owner.stable_key())
        else {
            return OwnedTerminalSummaryLookup::Absent;
        };
        if &summary.owner != owner {
            return OwnedTerminalSummaryLookup::OwnerMismatch;
        }
        OwnedTerminalSummaryLookup::Found(summary)
    }

    fn owned_summary(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<TerminalSurfaceSummary, UsecaseError> {
        match self.find_owned_summary(owner) {
            OwnedTerminalSummaryLookup::Found(summary) => Ok(summary),
            OwnedTerminalSummaryLookup::Absent | OwnedTerminalSummaryLookup::OwnerMismatch => {
                Err(UsecaseError::Gateway(format!(
                    "Terminal Surface not found for owner {}",
                    owner.stable_key()
                )))
            }
        }
    }

    pub(crate) fn get_summary(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<TerminalSurfaceSummary, UsecaseError> {
        self.owned_summary(owner)
    }

    pub(crate) fn get(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<TerminalSurface, UsecaseError> {
        let session_key = owner.stable_key();
        let registered_surface = self.owned_summary(owner)?;
        self.gateway
            .snapshot(registered_surface.runtime_generation.value())
            .ok_or_else(|| {
                UsecaseError::Gateway(format!(
                    "Terminal Surface not found for owner {session_key}"
                ))
            })
    }

    pub(crate) fn get_or_spawn(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwner,
        label: Option<String>,
        startup_command: Option<String>,
    ) -> Result<GetOrSpawnTerminalOutcome, UsecaseError> {
        let _admission = self.admit_mutation()?;
        super::spawn_usecase::get_or_spawn_with_startup(
            self.performance.as_ref(),
            self.gateway.as_ref(),
            rows,
            cols,
            cwd,
            owner,
            label,
            startup_command,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn get_or_spawn_process(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwner,
        label: Option<String>,
        process: TerminalProcessLaunch,
    ) -> Result<GetOrSpawnTerminalOutcome, UsecaseError> {
        let _admission = self.admit_mutation()?;
        super::spawn_usecase::get_or_spawn_with_process(
            self.performance.as_ref(),
            self.gateway.as_ref(),
            rows,
            cols,
            cwd,
            owner,
            label,
            process,
        )
    }

    pub(crate) fn write(
        &self,
        owner: &TerminalSurfaceOwner,
        data: &str,
    ) -> Result<(), UsecaseError> {
        let _admission = self.admit_mutation()?;
        super::io_usecase::write(self.gateway.as_ref(), owner, data)
    }

    pub(crate) fn write_attached(
        &self,
        owner: &TerminalSurfaceOwner,
        attachment_id: &str,
        sequence: u64,
        client_started_at_unix_ms: Option<f64>,
        data: &str,
    ) -> Result<(), UsecaseError> {
        if let Some(client_started_at_unix_ms) = client_started_at_unix_ms {
            self.performance.start_terminal_input_trace(
                attachment_id,
                sequence,
                client_started_at_unix_ms,
            );
        }
        let _admission = self.admit_mutation()?;
        self.performance
            .record_terminal_input_admission(attachment_id, sequence);
        self.gateway
            .write_attached(&owner.stable_key(), attachment_id, sequence, data)
            .map_err(|error| UsecaseError::Gateway(error.to_string()))
    }

    pub(crate) fn write_paths(
        &self,
        owner: &TerminalSurfaceOwner,
        paths: &[String],
    ) -> Result<(), UsecaseError> {
        let _admission = self.admit_mutation()?;
        super::io_usecase::write_paths(self.gateway.as_ref(), owner, paths)
    }

    pub(crate) fn resize(
        &self,
        owner: &TerminalSurfaceOwner,
        rows: u16,
        cols: u16,
    ) -> Result<(), UsecaseError> {
        self.prepare_resize(owner.clone(), rows, cols)()
    }

    pub(crate) fn prepare_resize(
        &self,
        owner: TerminalSurfaceOwner,
        rows: u16,
        cols: u16,
    ) -> impl FnOnce() -> Result<(), UsecaseError> + Send + 'static {
        let (done, next) = std::sync::mpsc::channel();
        let previous = self
            .resize_tails
            .lock()
            .unwrap()
            .insert(owner.stable_key(), next);
        let application = self.clone();
        let completion = TerminalSurfaceResizeCompletion {
            application: application.clone(),
            session_key: owner.stable_key(),
            done: Some(done),
        };
        move || {
            let _completion = completion;
            if let Some(previous) = previous {
                let _ = previous.recv();
            }
            let _admission = application.admit_mutation()?;
            super::io_usecase::resize(application.gateway.as_ref(), &owner, rows, cols)
        }
    }

    pub(crate) fn kill(&self, owner: &TerminalSurfaceOwner) -> Result<(), UsecaseError> {
        let _admission = self.admit_mutation()?;
        super::lifecycle_usecase::kill(self.gateway.as_ref(), owner)
    }

    pub(crate) fn stop_preserving_checkpoint(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<(), UsecaseError> {
        let _admission = self.admit_mutation()?;
        super::lifecycle_usecase::stop_preserving_checkpoint(self.gateway.as_ref(), owner)
    }

    pub(crate) fn delete_surface(&self, owner: &TerminalSurfaceOwner) -> Result<(), UsecaseError> {
        let _admission = self.admit_mutation()?;
        super::lifecycle_usecase::delete(self.gateway.as_ref(), owner)
    }

    pub(crate) fn shutdown(&self) -> Result<(), UsecaseError> {
        let mut lifecycle = self
            .runtime_lifecycle
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        lifecycle.begin_shutdown();

        let surfaces = self.gateway.list_summaries();
        let mut drain_targets = Vec::with_capacity(surfaces.len());
        let mut first_error = None;
        for surface in surfaces {
            let runtime_generation = surface.runtime_generation.value();
            if surface.process_state.is_exited() {
                drain_targets.push(runtime_generation);
                continue;
            }
            match self.gateway.request_runtime_stop(runtime_generation) {
                Ok(()) => drain_targets.push(runtime_generation),
                Err(error) => {
                    log::error!(
                        "application shutdown: terminal {runtime_generation} stop failed: {error}"
                    );
                    first_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
        for runtime_generation in drain_targets {
            if let Err(error) = self.gateway.wait_runtime_output_drain(runtime_generation) {
                log::error!(
                    "application shutdown: terminal {runtime_generation} drain failed: {error}"
                );
                first_error.get_or_insert_with(|| error.to_string());
            }
        }
        if let Err(error) = self.gateway.flush_checkpoints() {
            log::error!("application shutdown: terminal checkpoint flush failed: {error}");
            first_error.get_or_insert_with(|| error.to_string());
        }
        match first_error {
            Some(error) => Err(UsecaseError::Gateway(error)),
            None => Ok(()),
        }
    }

    pub(crate) fn flush_checkpoints(&self) -> Result<(), UsecaseError> {
        self.gateway
            .flush_checkpoints()
            .map_err(|error| UsecaseError::Gateway(error.to_string()))
    }

    pub(crate) fn kill_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
        let Ok(_admission) = self.admit_mutation() else {
            return Vec::new();
        };
        super::lifecycle_usecase::kill_by_worktree(self.gateway.as_ref(), worktree_path)
    }
}

#[cfg(test)]
#[path = "application_test.rs"]
mod application_tests;
