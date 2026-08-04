use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use tauri::Wry;
use tauri::{AppHandle, Runtime};

use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceRegistry, TerminalSurfaceSpawnReservation,
    TerminalSurfaceSpawnReservationError, TerminalSurfaceSummary,
};
use crate::domain::terminal_surface::gateway::{
    TerminalRuntimeSpawnRequest, TerminalSurfaceEvent, TerminalSurfaceEventSink,
    TerminalSurfaceGateway, TerminalSurfaceGatewayError, TerminalSurfaceRepository,
};
use crate::domain::terminal_surface::{
    TerminalSurfaceCheckpoint as DomainTerminalCheckpoint, TERMINAL_SURFACE_SCROLLBACK_ROWS,
};
use crate::infrastructure::terminal::checkpoint_scheduler::DirtyCheckpointScheduler;
#[cfg(test)]
use crate::infrastructure::terminal::native_pty::NativePtyResizer;
use crate::infrastructure::terminal::native_pty::{
    NativePtyOutput, NativePtyRuntime, NativePtySpawnConfig, NativePtySystem,
};
use crate::infrastructure::terminal::terminal_emulator::{
    NativeTerminalCheckpoint, NativeTerminalEmulator, TerminalCheckpointFileStore,
};
use crate::infrastructure::terminal::utf8_decoder::decode_utf8_chunk;

pub(crate) struct AttachedTerminalRuntime {
    native_pty: NativePtyRuntime,
    output: Option<NativePtyOutput>,
    event_order: Arc<TerminalSurfaceEventOrder>,
    terminal_surface: Arc<Mutex<NativeTerminalEmulator>>,
    checkpoint_scheduler: Option<DirtyCheckpointScheduler>,
    session_key: String,
    output_drained: Arc<(Mutex<bool>, Condvar)>,
}

#[cfg(test)]
pub type TerminalSurfaceRuntimeGateway = TerminalSurfaceRuntimeGatewayFor<Wry>;

const CHECKPOINT_PERSIST_INTERVAL: Duration = Duration::from_millis(250);

pub struct TerminalSurfaceRuntimeGatewayFor<R: Runtime> {
    app: Option<AppHandle<R>>,
    event_sink: Option<Arc<dyn TerminalSurfaceEventSink>>,
    registry: Arc<Mutex<TerminalSurfaceRegistry>>,
    spawn_resolved: Condvar,
    runtimes: Mutex<HashMap<u64, AttachedTerminalRuntime>>,
    native_pty: NativePtySystem,
    #[cfg(test)]
    snapshot_materialization_count: std::sync::atomic::AtomicUsize,
    runtime: PhantomData<fn() -> R>,
}

#[cfg(test)]
impl<R: Runtime> Default for TerminalSurfaceRuntimeGatewayFor<R> {
    fn default() -> Self {
        Self {
            app: None,
            event_sink: None,
            registry: Arc::new(Mutex::new(TerminalSurfaceRegistry::default())),
            spawn_resolved: Condvar::new(),
            runtimes: Mutex::new(HashMap::new()),
            native_pty: NativePtySystem,
            snapshot_materialization_count: std::sync::atomic::AtomicUsize::new(0),
            runtime: PhantomData,
        }
    }
}

fn process_pty_output(raw_chunk: &[u8], pending: &mut Vec<u8>) -> Option<String> {
    let raw = decode_utf8_chunk(raw_chunk, pending)?;
    let result = crate::infrastructure::terminal::shell_integration::strip_osc_cmd_done(&raw);

    if result.filtered_output.is_empty() {
        return None;
    }

    Some(result.filtered_output)
}

fn to_domain_checkpoint(checkpoint: &NativeTerminalCheckpoint) -> DomainTerminalCheckpoint {
    DomainTerminalCheckpoint {
        replay: checkpoint.replay.clone(),
        sequence: checkpoint.sequence,
        cols: checkpoint.cols,
        rows: checkpoint.rows,
    }
}

fn into_domain_checkpoint(checkpoint: NativeTerminalCheckpoint) -> DomainTerminalCheckpoint {
    DomainTerminalCheckpoint {
        replay: checkpoint.replay,
        sequence: checkpoint.sequence,
        cols: checkpoint.cols,
        rows: checkpoint.rows,
    }
}

fn materialize_surface(
    registry: &Arc<Mutex<TerminalSurfaceRegistry>>,
    runtime_generation: u64,
    terminal_surface: &Arc<Mutex<NativeTerminalEmulator>>,
) -> Option<TerminalSurface> {
    let terminal_surface = terminal_surface.lock();
    let mut registry = registry.lock();
    let sequence = registry.get(runtime_generation)?.latest_sequence();
    let checkpoint = terminal_surface.snapshot(sequence);
    registry.apply_checkpoint(runtime_generation, into_domain_checkpoint(checkpoint));
    registry.get(runtime_generation).cloned()
}

fn materialize_checkpoint(
    registry: &Arc<Mutex<TerminalSurfaceRegistry>>,
    runtime_generation: u64,
    terminal_surface: &Arc<Mutex<NativeTerminalEmulator>>,
) -> Option<NativeTerminalCheckpoint> {
    let terminal_surface = terminal_surface.lock();
    let mut registry = registry.lock();
    let sequence = registry.get(runtime_generation)?.latest_sequence();
    let checkpoint = terminal_surface.snapshot(sequence);
    registry.apply_checkpoint(runtime_generation, to_domain_checkpoint(&checkpoint));
    Some(checkpoint)
}

#[derive(Default)]
struct TerminalSurfaceEventOrder {
    serialization: Mutex<()>,
}

impl TerminalSurfaceEventOrder {
    fn advance_and_publish<T>(
        &self,
        event_sink: Option<&dyn TerminalSurfaceEventSink>,
        advance: impl FnOnce() -> Option<(T, TerminalSurfaceEvent)>,
    ) -> Option<T> {
        let _serialization = self.serialization.lock();
        let (result, event) = advance()?;
        if let Some(event_sink) = event_sink {
            event_sink.publish(event);
        }
        Some(result)
    }
}

struct TerminalOutputReaderContext {
    event_sink: Option<Arc<dyn TerminalSurfaceEventSink>>,
    event_order: Arc<TerminalSurfaceEventOrder>,
    registry: Arc<Mutex<TerminalSurfaceRegistry>>,
    runtime_generation: u64,
    terminal_surface: Arc<Mutex<NativeTerminalEmulator>>,
    checkpoint_scheduler: Option<DirtyCheckpointScheduler>,
    session_key: String,
    output_drained: Arc<(Mutex<bool>, Condvar)>,
}

fn spawn_output_reader(mut output: NativePtyOutput, context: TerminalOutputReaderContext) {
    let TerminalOutputReaderContext {
        event_sink,
        event_order,
        registry,
        runtime_generation,
        terminal_surface,
        checkpoint_scheduler,
        session_key,
        output_drained,
    } = context;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut pending = Vec::new();
        loop {
            match output.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(filtered) = process_pty_output(&buf[..n], &mut pending) {
                        let sequence =
                            event_order.advance_and_publish(event_sink.as_deref(), || {
                                let sequence = {
                                    let mut terminal_surface = terminal_surface.lock();
                                    terminal_surface.apply(&filtered);
                                    registry.lock().record_output(runtime_generation)?
                                };
                                if let Some(checkpoint_scheduler) = &checkpoint_scheduler {
                                    checkpoint_scheduler.mark_dirty();
                                }
                                Some((
                                    sequence,
                                    TerminalSurfaceEvent::Output {
                                        session_key: session_key.clone(),
                                        data: filtered,
                                        sequence,
                                    },
                                ))
                            });
                        if sequence.is_none() {
                            continue;
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let exit_code = output.wait().ok().flatten();
        event_order.advance_and_publish(event_sink.as_deref(), || {
            let sequence = {
                let terminal_surface = terminal_surface.lock();
                let mut registry = registry.lock();
                let sequence = registry.mark_exited(runtime_generation, exit_code)?;
                if checkpoint_scheduler.is_none() {
                    let checkpoint = terminal_surface.snapshot(sequence);
                    registry
                        .apply_checkpoint(runtime_generation, into_domain_checkpoint(checkpoint));
                }
                sequence
            };
            if let Some(checkpoint_scheduler) = &checkpoint_scheduler {
                if let Err(error) = checkpoint_scheduler.flush() {
                    log::error!("failed to persist final Terminal Surface: {error}");
                }
            }
            Some((
                sequence,
                TerminalSurfaceEvent::Exit {
                    session_key,
                    exit_code,
                    sequence,
                },
            ))
        });
        let (drained, changed) = &*output_drained;
        *drained.lock() = true;
        changed.notify_all();
    });
}

fn wait_for_output_drain(output_drained: &Arc<(Mutex<bool>, Condvar)>) {
    let (drained, changed) = &**output_drained;
    let mut drained = drained.lock();
    while !*drained {
        changed.wait(&mut drained);
    }
}

impl<R: Runtime> TerminalSurfaceRuntimeGatewayFor<R> {
    #[cfg(test)]
    pub fn new(app: AppHandle<R>) -> Self {
        Self {
            app: Some(app),
            event_sink: None,
            registry: Arc::new(Mutex::new(TerminalSurfaceRegistry::default())),
            spawn_resolved: Condvar::new(),
            runtimes: Mutex::new(HashMap::new()),
            native_pty: NativePtySystem,
            snapshot_materialization_count: std::sync::atomic::AtomicUsize::new(0),
            runtime: PhantomData,
        }
    }

    pub fn new_with_event_sink(
        app: AppHandle<R>,
        event_sink: Arc<dyn TerminalSurfaceEventSink>,
    ) -> Self {
        Self {
            app: Some(app),
            event_sink: Some(event_sink),
            registry: Arc::new(Mutex::new(TerminalSurfaceRegistry::default())),
            spawn_resolved: Condvar::new(),
            runtimes: Mutex::new(HashMap::new()),
            native_pty: NativePtySystem,
            #[cfg(test)]
            snapshot_materialization_count: std::sync::atomic::AtomicUsize::new(0),
            runtime: PhantomData,
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot_materialization_count(&self) -> usize {
        self.snapshot_materialization_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn app(&self) -> Result<&AppHandle<R>, TerminalSurfaceGatewayError> {
        self.app
            .as_ref()
            .ok_or_else(|| TerminalSurfaceGatewayError::new("Terminal runtime host is not bound"))
    }

    fn runtime_generation_for_session_key(&self, session_key: &str) -> Option<u64> {
        self.registry
            .lock()
            .find_by_session_key(session_key)
            .map(|surface| surface.runtime_generation.value())
    }

    fn materialize_surface(&self, runtime_generation: u64) -> Option<TerminalSurface> {
        let terminal_surface = self
            .runtimes
            .lock()
            .get(&runtime_generation)
            .map(|runtime| Arc::clone(&runtime.terminal_surface));
        let Some(terminal_surface) = terminal_surface else {
            return self.registry.lock().get(runtime_generation).cloned();
        };
        materialize_surface(&self.registry, runtime_generation, &terminal_surface)
    }
}

impl<R: Runtime> TerminalSurfaceRepository for TerminalSurfaceRuntimeGatewayFor<R> {
    fn find_summary_by_session_key(&self, session_key: &str) -> Option<TerminalSurfaceSummary> {
        self.registry
            .lock()
            .find_by_session_key(session_key)
            .map(TerminalSurface::summary)
    }

    fn list_summaries(&self) -> Vec<TerminalSurfaceSummary> {
        self.registry.lock().list_summaries()
    }
}

impl<R: Runtime> TerminalSurfaceGateway for TerminalSurfaceRuntimeGatewayFor<R> {
    fn next_runtime_generation(&self) -> u64 {
        self.registry.lock().next_runtime_generation()
    }

    fn load_terminal_checkpoint(
        &self,
        session_key: &str,
    ) -> Result<Option<DomainTerminalCheckpoint>, TerminalSurfaceGatewayError> {
        let Some(app) = self.app.as_ref() else {
            return Ok(None);
        };
        let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(TerminalSurfaceGatewayError::new)?;
        TerminalCheckpointFileStore::new(&data_dir)
            .load(session_key)
            .map(|checkpoint| checkpoint.map(into_domain_checkpoint))
            .map_err(TerminalSurfaceGatewayError::new)
    }

    fn delete_terminal_checkpoint(
        &self,
        session_key: &str,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let Some(app) = self.app.as_ref() else {
            return Ok(());
        };
        let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(TerminalSurfaceGatewayError::new)?;
        TerminalCheckpointFileStore::new(&data_dir)
            .delete(session_key)
            .map_err(TerminalSurfaceGatewayError::new)
    }

    fn spawn_runtime(
        &self,
        request: TerminalRuntimeSpawnRequest,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let app = self.app()?;
        let runtime_generation = request.runtime_generation;
        let app_data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(TerminalSurfaceGatewayError::new)?;
        let integration_dir = if request.exec_command.is_some() {
            None
        } else {
            crate::infrastructure::terminal::shell_integration::create_shell_integration_files(
                &app_data_dir,
            )
            .ok()
        };
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        let mut extra_env = Vec::new();
        match crate::infrastructure::platform::path_aliases::prepare_child_env(Some(
            app_data_dir.clone(),
        )) {
            Ok(env) => extra_env.extend(env),
            Err(e) => {
                return Err(TerminalSurfaceGatewayError::new(format!(
                    "failed to prepare alias child env for PTY spawn: {e}"
                )));
            }
        }
        let backend_session = self
            .native_pty
            .spawn(NativePtySpawnConfig {
                rows: request.rows,
                cols: request.cols,
                cwd: request.cwd,
                shell,
                integration_dir,
                runtime_id: runtime_generation,
                extra_env,
                exec_command: request.exec_command,
            })
            .map_err(TerminalSurfaceGatewayError::new)?;

        let initial_sequence = request
            .initial_terminal_surface
            .as_ref()
            .map_or(0, |checkpoint| checkpoint.sequence);
        let terminal_surface = request.initial_terminal_surface.as_ref().map_or_else(
            || {
                NativeTerminalEmulator::new(
                    request.cols,
                    request.rows,
                    TERMINAL_SURFACE_SCROLLBACK_ROWS,
                )
            },
            |checkpoint| {
                NativeTerminalEmulator::restore(
                    &NativeTerminalCheckpoint {
                        replay: checkpoint.replay.clone(),
                        sequence: checkpoint.sequence,
                        cols: checkpoint.cols,
                        rows: checkpoint.rows,
                    },
                    TERMINAL_SURFACE_SCROLLBACK_ROWS,
                )
            },
        );
        let checkpoint_store = TerminalCheckpointFileStore::new(&app_data_dir);
        if let Err(error) = checkpoint_store.save(
            &request.session_key,
            &terminal_surface.snapshot(initial_sequence),
        ) {
            let _ = backend_session.runtime.kill();
            return Err(TerminalSurfaceGatewayError::new(format!(
                "failed to persist initial Terminal Surface: {error}"
            )));
        }
        let terminal_surface = Arc::new(Mutex::new(terminal_surface));
        let checkpoint_scheduler = Some({
            let store = checkpoint_store;
            let registry = Arc::clone(&self.registry);
            let terminal_surface = Arc::clone(&terminal_surface);
            let session_key = request.session_key.clone();
            DirtyCheckpointScheduler::spawn(
                CHECKPOINT_PERSIST_INTERVAL,
                Arc::new(move || {
                    let checkpoint =
                        materialize_checkpoint(&registry, runtime_generation, &terminal_surface)
                            .ok_or_else(|| {
                                format!("Terminal Surface for PTY {runtime_generation} not found")
                            })?;
                    store.save(&session_key, &checkpoint)
                }),
            )
        });
        let runtime = AttachedTerminalRuntime {
            native_pty: backend_session.runtime,
            output: Some(backend_session.output),
            event_order: Arc::new(TerminalSurfaceEventOrder::default()),
            terminal_surface,
            checkpoint_scheduler,
            session_key: request.session_key,
            output_drained: Arc::new((Mutex::new(false), Condvar::new())),
        };
        let mut runtimes = self.runtimes.lock();
        if runtimes.contains_key(&runtime_generation) {
            return Err(TerminalSurfaceGatewayError::new(format!(
                "PTY {runtime_generation} already exists"
            )));
        }
        runtimes.insert(runtime_generation, runtime);
        Ok(())
    }

    fn insert_surface(&self, surface: TerminalSurface) {
        let active_count = {
            let mut registry = self.registry.lock();
            registry.insert(surface);
            registry.len()
        };
        crate::other::telemetry::set_active_pty_count(active_count as u64);
    }

    fn start_output_reader(
        &self,
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let (
            output,
            event_order,
            terminal_surface,
            checkpoint_scheduler,
            session_key,
            output_drained,
        ) = {
            let mut runtimes = self.runtimes.lock();
            let runtime = runtimes.get_mut(&runtime_generation).ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!("PTY {} not found", runtime_generation))
            })?;
            let output = runtime.output.take().ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!(
                    "PTY {} output reader already started",
                    runtime_generation
                ))
            })?;
            (
                output,
                Arc::clone(&runtime.event_order),
                Arc::clone(&runtime.terminal_surface),
                runtime.checkpoint_scheduler.clone(),
                runtime.session_key.clone(),
                Arc::clone(&runtime.output_drained),
            )
        };
        spawn_output_reader(
            output,
            TerminalOutputReaderContext {
                event_sink: self.event_sink.clone(),
                event_order,
                registry: Arc::clone(&self.registry),
                runtime_generation,
                terminal_surface,
                checkpoint_scheduler,
                session_key,
                output_drained,
            },
        );
        Ok(())
    }

    fn snapshot(&self, runtime_generation: u64) -> Option<TerminalSurface> {
        #[cfg(test)]
        self.snapshot_materialization_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.materialize_surface(runtime_generation)
    }

    fn select_kill_targets_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
        self.registry
            .lock()
            .select_kill_targets_by_worktree(worktree_path)
    }

    fn select_gc_targets(&self, worktree_path: &str, keep_session_keys: &[String]) -> Vec<u64> {
        self.registry
            .lock()
            .select_gc_targets(worktree_path, keep_session_keys)
    }

    fn remove_surface(&self, runtime_generation: u64) -> Option<TerminalSurface> {
        self.runtimes.lock().remove(&runtime_generation);
        let (removed, active_count) = {
            let mut registry = self.registry.lock();
            let removed = registry.remove(runtime_generation);
            (removed, registry.len())
        };
        crate::other::telemetry::set_active_pty_count(active_count as u64);
        removed
    }

    fn reserve_spawn_slot(
        &self,
        session_key: &str,
        worktree_path: Option<&str>,
    ) -> Result<TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError> {
        self.registry
            .lock()
            .reserve_spawn_slot(session_key, worktree_path)
    }

    fn complete_spawn_slot(&self, reservation: &TerminalSurfaceSpawnReservation) {
        self.registry.lock().complete_spawn_slot(reservation);
        self.spawn_resolved.notify_all();
    }

    fn rollback_spawn_slot(&self, reservation: &TerminalSurfaceSpawnReservation) {
        self.registry.lock().rollback_spawn_slot(reservation);
        self.spawn_resolved.notify_all();
    }

    fn wait_for_spawn_resolution(&self, session_key: &str) -> Option<TerminalSurfaceSummary> {
        let mut registry = self.registry.lock();
        loop {
            if let Some(surface) = registry.find_by_session_key(session_key) {
                return Some(surface.summary());
            }
            if !registry.is_spawn_reserved(session_key) {
                return None;
            }
            self.spawn_resolved.wait(&mut registry);
        }
    }

    fn write(&self, session_key: &str, data: &str) -> Result<(), TerminalSurfaceGatewayError> {
        let runtime_generation = self
            .runtime_generation_for_session_key(session_key)
            .ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!(
                    "Terminal Surface not found for owner {session_key}"
                ))
            })?;
        {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes.get(&runtime_generation).ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!("PTY {} not found", runtime_generation))
            })?;
            runtime
                .native_pty
                .write(data.as_bytes())
                .map_err(TerminalSurfaceGatewayError::new)?;
        }
        Ok(())
    }

    fn resize(
        &self,
        session_key: &str,
        rows: u16,
        cols: u16,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        if rows == 0 || cols == 0 {
            return Ok(());
        }
        let runtime_generation = self
            .runtime_generation_for_session_key(session_key)
            .ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!(
                    "Terminal Surface not found for owner {session_key}"
                ))
            })?;
        let (native_pty, event_order, terminal_surface, checkpoint_scheduler, session_key) = {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes.get(&runtime_generation).ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!("PTY {} not found", runtime_generation))
            })?;
            (
                runtime.native_pty.clone(),
                Arc::clone(&runtime.event_order),
                Arc::clone(&runtime.terminal_surface),
                runtime.checkpoint_scheduler.clone(),
                runtime.session_key.clone(),
            )
        };
        let _serialization = event_order.serialization.lock();
        native_pty
            .resize(rows, cols)
            .map_err(TerminalSurfaceGatewayError::new)?;
        let sequence = {
            let mut terminal_surface = terminal_surface.lock();
            terminal_surface.resize(cols, rows);
            self.registry
                .lock()
                .record_resize(runtime_generation)
                .ok_or_else(|| {
                    TerminalSurfaceGatewayError::new(format!(
                        "Terminal Surface for PTY {runtime_generation} not found"
                    ))
                })?
        };
        if let Some(checkpoint_scheduler) = checkpoint_scheduler {
            checkpoint_scheduler.mark_dirty();
        }
        if let Some(event_sink) = self.event_sink.as_deref() {
            event_sink.publish(TerminalSurfaceEvent::Resize {
                session_key,
                cols,
                rows,
                sequence,
            });
        }
        debug_assert!(sequence > 0);
        Ok(())
    }

    fn request_runtime_stop(
        &self,
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let native_pty = {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes.get(&runtime_generation).ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!("PTY {} not found", runtime_generation))
            })?;
            runtime.native_pty.clone()
        };
        native_pty.kill().map_err(|error| {
            TerminalSurfaceGatewayError::new(format!("PTY {runtime_generation}: {error}"))
        })
    }

    fn wait_runtime_output_drain(
        &self,
        runtime_generation: u64,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let output_drained = self
            .runtimes
            .lock()
            .get(&runtime_generation)
            .map(|runtime| Arc::clone(&runtime.output_drained))
            .ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!("PTY {} not found", runtime_generation))
            })?;
        wait_for_output_drain(&output_drained);
        Ok(())
    }

    fn remove_runtime(&self, runtime_generation: u64) {
        self.runtimes.lock().remove(&runtime_generation);
    }

    fn flush_checkpoints(&self) -> Result<(), TerminalSurfaceGatewayError> {
        let schedulers = self
            .runtimes
            .lock()
            .values()
            .filter_map(|runtime| runtime.checkpoint_scheduler.clone())
            .collect::<Vec<_>>();
        for scheduler in schedulers {
            scheduler
                .flush()
                .map_err(TerminalSurfaceGatewayError::new)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "runtime_gateway_impl_test.rs"]
mod runtime_gateway_impl_tests;
