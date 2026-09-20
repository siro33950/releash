use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceAttachment, TerminalSurfaceMutationRejected,
    TerminalSurfaceRuntimeLifecycle, TerminalSurfaceSequenceDecision, TerminalSurfaceSummary,
};
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventCancellation, TerminalSurfaceEventReceiveError,
    TerminalSurfaceEventSource, TerminalSurfaceEventSubscription, TerminalSurfaceGateway,
    TerminalSurfaceInputUnavailableCause,
};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::usecase::terminal_surface::error::UsecaseError;
use crate::usecase::terminal_surface::spawn_usecase::GetOrSpawnTerminalOutcome;

#[derive(Clone)]
pub(crate) struct TerminalSurfaceApplication {
    gateway: Arc<dyn TerminalSurfaceGateway + Send + Sync>,
    event_source: Arc<dyn TerminalSurfaceEventSource>,
    attach_lock: Arc<Mutex<()>>,
    attachment_cancellations: Arc<Mutex<HashMap<String, TerminalSurfaceAttachmentRegistration>>>,
    runtime_lifecycle: Arc<RwLock<TerminalSurfaceRuntimeLifecycle>>,
    resize_tails: Arc<Mutex<HashMap<String, std::sync::mpsc::Receiver<()>>>>,
}

struct TerminalSurfaceAttachmentRegistration {
    session_key: String,
    cancellation: Arc<dyn TerminalSurfaceEventCancellation>,
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

pub(crate) struct TerminalSurfaceAttachmentStream {
    application: TerminalSurfaceApplication,
    owner: TerminalSurfaceOwner,
    session_key: String,
    attachment: TerminalSurfaceAttachment,
    pending_snapshot: Option<TerminalSurface>,
    subscription: Box<dyn TerminalSurfaceEventSubscription>,
    cancellation: Arc<dyn TerminalSurfaceEventCancellation>,
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
    InputUnavailable {
        session_key: String,
        cause: TerminalSurfaceInputUnavailableCause,
    },
}

impl Drop for TerminalSurfaceAttachmentStream {
    fn drop(&mut self) {
        let mut registrations = self
            .application
            .attachment_cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = self.attachment.attachment_id();
        if registrations
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(&current.cancellation, &self.cancellation))
        {
            if let Some(registration) = registrations.remove(id) {
                registration.cancellation.cancel();
                self.application
                    .gateway
                    .deactivate_input_attachment(&registration.session_key, id);
            }
        }
    }
}

impl TerminalSurfaceAttachmentStream {
    pub(crate) fn should_resynchronize(&self) -> bool {
        self.attachment.should_resynchronize()
    }

    fn resynchronize(
        &mut self,
        minimum_covered_sequence: Option<u64>,
    ) -> Option<TerminalSurfaceStreamItem> {
        let surface = match self.application.get(&self.owner) {
            Ok(surface) => surface,
            Err(_) => {
                self.attachment.close();
                return None;
            }
        };
        if !self.attachment.apply_snapshot(
            surface.checkpoint.sequence,
            minimum_covered_sequence,
            surface.process_state.is_exited(),
        ) {
            return None;
        }
        Some(TerminalSurfaceStreamItem::Snapshot(surface))
    }

    pub(crate) async fn next(&mut self) -> Option<TerminalSurfaceStreamItem> {
        if let Some(surface) = self.pending_snapshot.take() {
            self.attachment.apply_snapshot(
                surface.checkpoint.sequence,
                None,
                surface.process_state.is_exited(),
            );
            return Some(TerminalSurfaceStreamItem::Snapshot(surface));
        }
        if self.attachment.is_closed() {
            return None;
        }

        loop {
            let received = self.subscription.recv().await;
            match received {
                Ok(TerminalSurfaceEvent::Output {
                    session_key,
                    data,
                    sequence,
                }) if session_key == self.session_key => {
                    match self.attachment.observe(sequence, false) {
                        TerminalSurfaceSequenceDecision::Deliver => {
                            return Some(TerminalSurfaceStreamItem::Output {
                                session_key,
                                data,
                                sequence,
                            });
                        }
                        TerminalSurfaceSequenceDecision::Ignore => {}
                        TerminalSurfaceSequenceDecision::Resynchronize => {
                            return self.resynchronize(Some(sequence));
                        }
                        TerminalSurfaceSequenceDecision::Closed => return None,
                    }
                }
                Ok(TerminalSurfaceEvent::Resize {
                    session_key,
                    cols,
                    rows,
                    sequence,
                }) if session_key == self.session_key => {
                    match self.attachment.observe(sequence, false) {
                        TerminalSurfaceSequenceDecision::Deliver => {
                            return Some(TerminalSurfaceStreamItem::Resize {
                                session_key,
                                cols,
                                rows,
                                sequence,
                            });
                        }
                        TerminalSurfaceSequenceDecision::Ignore => {}
                        TerminalSurfaceSequenceDecision::Resynchronize => {
                            return self.resynchronize(Some(sequence));
                        }
                        TerminalSurfaceSequenceDecision::Closed => return None,
                    }
                }
                Ok(TerminalSurfaceEvent::Exit {
                    session_key,
                    exit_code,
                    sequence,
                    ..
                }) if session_key == self.session_key => {
                    match self.attachment.observe(sequence, true) {
                        TerminalSurfaceSequenceDecision::Deliver => {
                            return Some(TerminalSurfaceStreamItem::Exit {
                                session_key,
                                exit_code,
                                sequence,
                            });
                        }
                        TerminalSurfaceSequenceDecision::Ignore => {}
                        TerminalSurfaceSequenceDecision::Resynchronize => {
                            return self.resynchronize(Some(sequence));
                        }
                        TerminalSurfaceSequenceDecision::Closed => return None,
                    }
                }
                Ok(TerminalSurfaceEvent::InputUnavailable { session_key, cause })
                    if session_key == self.session_key =>
                {
                    return Some(TerminalSurfaceStreamItem::InputUnavailable {
                        session_key,
                        cause,
                    });
                }
                Ok(_) => {}
                Err(TerminalSurfaceEventReceiveError::Lagged(_)) => {
                    return self.resynchronize(None);
                }
                Err(TerminalSurfaceEventReceiveError::Closed) => {
                    self.attachment.close();
                    return None;
                }
            }
        }
    }
}

impl TerminalSurfaceApplication {
    fn mutation_rejected(_: TerminalSurfaceMutationRejected) -> UsecaseError {
        UsecaseError::Gateway("Terminal Surface runtime is shutting down".to_string())
    }

    pub(crate) fn new(
        gateway: Arc<dyn TerminalSurfaceGateway + Send + Sync>,
        event_source: Arc<dyn TerminalSurfaceEventSource>,
    ) -> Self {
        Self {
            gateway,
            event_source,
            attach_lock: Arc::new(Mutex::new(())),
            resize_tails: Arc::new(Mutex::new(HashMap::new())),
            attachment_cancellations: Arc::new(Mutex::new(HashMap::new())),
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

    pub(crate) fn attach(
        &self,
        attachment_id: &str,
        owner: &TerminalSurfaceOwner,
    ) -> Result<TerminalSurfaceAttachmentStream, UsecaseError> {
        if attachment_id.trim().is_empty() {
            return Err(UsecaseError::Gateway(
                "Terminal Surface attachment id must not be empty".to_string(),
            ));
        }
        // ponytail: attach全体は直列化する。並列snapshotが必要になればattachment単位へ分割する。
        let _attach = self
            .attach_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let event_stream = self
            .event_source
            .subscribe_owner(&owner.stable_key(), attachment_id);
        let surface = self.get(owner)?;
        let mut registrations = self
            .attachment_cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.gateway
            .activate_input_attachment(&surface.session_key, attachment_id);
        if let Some(previous) = registrations.insert(
            attachment_id.to_string(),
            TerminalSurfaceAttachmentRegistration {
                session_key: surface.session_key.clone(),
                cancellation: event_stream.cancellation.clone(),
            },
        ) {
            previous.cancellation.cancel();
            if previous.session_key != surface.session_key {
                self.gateway
                    .deactivate_input_attachment(&previous.session_key, attachment_id);
            }
        }
        Ok(TerminalSurfaceAttachmentStream {
            application: self.clone(),
            owner: owner.clone(),
            session_key: surface.session_key.clone(),
            attachment: TerminalSurfaceAttachment::new(
                attachment_id.to_string(),
                surface.checkpoint.sequence,
            ),
            pending_snapshot: Some(surface),
            subscription: event_stream.subscription,
            cancellation: event_stream.cancellation,
        })
    }

    pub(crate) fn detach(&self, attachment_id: &str) {
        if let Some(registration) = self
            .attachment_cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(attachment_id)
        {
            registration.cancellation.cancel();
            self.gateway
                .deactivate_input_attachment(&registration.session_key, attachment_id);
        }
    }

    pub(crate) fn acknowledge_output(&self, attachment_id: &str, sequence: u64) {
        let session_key = self
            .attachment_cancellations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(attachment_id)
            .map(|registration| registration.session_key.clone());
        if let Some(session_key) = session_key {
            self.event_source
                .acknowledge_owner_output(&session_key, attachment_id, sequence);
        }
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
            crate::other::telemetry::start_terminal_input_trace(
                attachment_id,
                sequence,
                client_started_at_unix_ms,
            );
        }
        let _admission = self.admit_mutation()?;
        crate::other::telemetry::record_terminal_input_admission(attachment_id, sequence);
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
