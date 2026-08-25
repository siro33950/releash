use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(test)]
use tauri::Wry;
use tauri::{AppHandle, Runtime};

use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceInputIngressError, TerminalSurfaceInputIngressRegistry,
    TerminalSurfaceRegistry, TerminalSurfaceSpawnReservation, TerminalSurfaceSpawnReservationError,
    TerminalSurfaceSummary,
};
use crate::domain::terminal_surface::gateway::{
    TerminalRuntimeSpawnRequest, TerminalSurfaceEvent, TerminalSurfaceEventSink,
    TerminalSurfaceGateway, TerminalSurfaceGatewayError, TerminalSurfaceInputUnavailableCause,
    TerminalSurfaceRepository,
};
use crate::domain::terminal_surface::{
    TerminalSurfaceCheckpoint as DomainTerminalCheckpoint, TerminalSurfaceLifecycleConfig,
    TERMINAL_SURFACE_SCROLLBACK_ROWS,
};
use crate::infrastructure::terminal::checkpoint_journal::IncrementalCheckpointJournal;
use crate::infrastructure::terminal::checkpoint_scheduler::DirtyCheckpointScheduler;
#[cfg(test)]
use crate::infrastructure::terminal::native_pty::NativePtyResizer;
use crate::infrastructure::terminal::native_pty::{
    NativePtyOutput, NativePtyProcessConfig, NativePtyRuntime, NativePtySpawnConfig,
    NativePtySystem,
};
use crate::infrastructure::terminal::output_batcher::TerminalOutputBatcher;
use crate::infrastructure::terminal::terminal_emulator::{
    NativeTerminalCheckpoint, NativeTerminalCheckpointRecord, NativeTerminalEmulator,
    TerminalCheckpointFileStore,
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
    checkpoint_journal: Option<Arc<Mutex<IncrementalCheckpointJournal>>>,
    checkpoint_store: Option<TerminalCheckpointFileStore>,
    checkpoint_io: Option<Arc<Mutex<()>>>,
    pending_input_traces: Arc<Mutex<VecDeque<crate::other::telemetry::TerminalInputTraceKey>>>,
}

#[cfg(test)]
pub type TerminalSurfaceRuntimeGateway = TerminalSurfaceRuntimeGatewayFor<Wry>;

const CHECKPOINT_PERSIST_INTERVAL: Duration = Duration::from_millis(250);

pub struct TerminalSurfaceRuntimeGatewayFor<R: Runtime> {
    app: Option<AppHandle<R>>,
    event_sink: Option<Arc<dyn TerminalSurfaceEventSink>>,
    registry: Arc<Mutex<TerminalSurfaceRegistry>>,
    input_ingress: Mutex<TerminalSurfaceInputIngressRegistry>,
    spawn_resolved: Condvar,
    runtimes: Mutex<HashMap<u64, AttachedTerminalRuntime>>,
    native_pty: NativePtySystem,
    journal_enabled: bool,
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
            input_ingress: Mutex::new(TerminalSurfaceInputIngressRegistry::default()),
            spawn_resolved: Condvar::new(),
            runtimes: Mutex::new(HashMap::new()),
            native_pty: NativePtySystem,
            journal_enabled: true,
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
    let checkpoint = materialize_checkpoint(registry, runtime_generation, terminal_surface).ok()?;
    let mut registry = registry.lock();
    registry.apply_checkpoint(runtime_generation, into_domain_checkpoint(checkpoint));
    registry.get(runtime_generation).cloned()
}

fn materialize_checkpoint(
    registry: &Arc<Mutex<TerminalSurfaceRegistry>>,
    runtime_generation: u64,
    terminal_surface: &Arc<Mutex<NativeTerminalEmulator>>,
) -> Result<NativeTerminalCheckpoint, String> {
    let terminal_surface = terminal_surface.lock();
    let registry = registry.lock();
    let sequence = registry
        .get(runtime_generation)
        .map(TerminalSurface::latest_sequence)
        .ok_or_else(|| format!("Terminal Surface for PTY {runtime_generation} not found"))?;
    Ok(terminal_surface.snapshot(sequence))
}

fn compact_checkpoint(
    store: &TerminalCheckpointFileStore,
    session_key: &str,
    registry: &Arc<Mutex<TerminalSurfaceRegistry>>,
    runtime_generation: u64,
    terminal_surface: &Arc<Mutex<NativeTerminalEmulator>>,
    journal: &Arc<Mutex<IncrementalCheckpointJournal>>,
) -> Result<(), String> {
    let checkpoint = materialize_checkpoint(registry, runtime_generation, terminal_surface)?;
    store.replace_base(session_key, &checkpoint)?;
    journal.lock().compacted(checkpoint.clone());
    registry
        .lock()
        .apply_checkpoint(runtime_generation, to_domain_checkpoint(&checkpoint));
    Ok(())
}

const CHECKPOINT_JOURNAL_COMPACTION_BYTES: u64 = 2 * 1024 * 1024;

fn flush_incremental_checkpoint(
    store: &TerminalCheckpointFileStore,
    session_key: &str,
    registry: &Arc<Mutex<TerminalSurfaceRegistry>>,
    runtime_generation: u64,
    terminal_surface: &Arc<Mutex<NativeTerminalEmulator>>,
    journal: &Arc<Mutex<IncrementalCheckpointJournal>>,
) -> Result<(), String> {
    let pending = journal.lock().take_pending();
    let persist_result = (|| {
        if let Some(base) = &pending.base {
            store.replace_base(session_key, base)?;
        }
        store.append_records(session_key, &pending.records)?;
        Ok(())
    })();
    if let Err(error) = persist_result {
        journal.lock().restore_failed(pending);
        return Err(error);
    }
    if store.journal_len(session_key)? >= CHECKPOINT_JOURNAL_COMPACTION_BYTES {
        compact_checkpoint(
            store,
            session_key,
            registry,
            runtime_generation,
            terminal_surface,
            journal,
        )?;
    }
    Ok(())
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
    checkpoint_journal: Option<Arc<Mutex<IncrementalCheckpointJournal>>>,
    journal_enabled: bool,
    first_provider_byte_started_at: Instant,
    pending_input_traces: Arc<Mutex<VecDeque<crate::other::telemetry::TerminalInputTraceKey>>>,
}

enum TerminalOutputCommand {
    Data {
        data: String,
        input_traces: Vec<crate::other::telemetry::TerminalInputTraceKey>,
    },
    Exit(Option<i32>),
}

const OUTPUT_READER_QUEUE_CAPACITY: usize = 256;

fn publish_terminal_output(
    context: &TerminalOutputReaderContext,
    data: String,
    input_traces: Vec<crate::other::telemetry::TerminalInputTraceKey>,
) {
    for trace in &input_traces {
        crate::other::telemetry::record_terminal_input_model_apply(trace);
    }
    let data: Arc<str> = Arc::from(data);
    let published = context
        .event_order
        .advance_and_publish(context.event_sink.as_deref(), || {
            let sequence = {
                let mut terminal_surface = context.terminal_surface.lock();
                terminal_surface.apply(&data);
                context
                    .registry
                    .lock()
                    .record_output(context.runtime_generation, Instant::now())?
            };
            if let Some(checkpoint_scheduler) = context
                .checkpoint_scheduler
                .as_ref()
                .filter(|_| context.journal_enabled)
            {
                if let Some(journal) = &context.checkpoint_journal {
                    if let Err(error) =
                        journal
                            .lock()
                            .record(NativeTerminalCheckpointRecord::Output {
                                sequence,
                                data: Arc::clone(&data),
                            })
                    {
                        log::error!("failed to collect Terminal Surface output: {error}");
                    }
                }
                checkpoint_scheduler.mark_dirty();
            }
            Some((
                sequence,
                TerminalSurfaceEvent::Output {
                    session_key: context.session_key.clone(),
                    data,
                    sequence,
                },
            ))
        });
    if published.is_some() {
        for trace in &input_traces {
            crate::other::telemetry::record_terminal_input_event_publish(trace);
        }
    }
}

fn publish_terminal_exit(context: &TerminalOutputReaderContext, exit_code: Option<i32>) {
    context
        .event_order
        .advance_and_publish(context.event_sink.as_deref(), || {
            let sequence = {
                let terminal_surface = context.terminal_surface.lock();
                let mut registry = context.registry.lock();
                let sequence = registry.mark_exited(context.runtime_generation, exit_code)?;
                if context.checkpoint_scheduler.is_none() {
                    let checkpoint = terminal_surface.snapshot(sequence);
                    registry.apply_checkpoint(
                        context.runtime_generation,
                        into_domain_checkpoint(checkpoint),
                    );
                }
                sequence
            };
            if let Some(checkpoint_scheduler) = &context.checkpoint_scheduler {
                if let Some(journal) = &context.checkpoint_journal {
                    if let Err(error) = journal
                        .lock()
                        .record(NativeTerminalCheckpointRecord::Barrier { sequence })
                    {
                        log::error!("failed to collect Terminal Surface exit: {error}");
                    }
                }
                if let Err(error) = checkpoint_scheduler.flush() {
                    log::error!("failed to persist final Terminal Surface: {error}");
                }
            }
            Some((
                sequence,
                TerminalSurfaceEvent::Exit {
                    session_key: context.session_key.clone(),
                    runtime_generation: context.runtime_generation,
                    exit_code,
                    sequence,
                },
            ))
        });
}

fn run_output_processor(
    receiver: mpsc::Receiver<TerminalOutputCommand>,
    context: TerminalOutputReaderContext,
) {
    let mut batcher = TerminalOutputBatcher::default();
    let mut pending_input_traces = Vec::new();
    loop {
        let command = match batcher.remaining_window(Instant::now()) {
            Some(wait) => receiver.recv_timeout(wait).map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => None,
                mpsc::RecvTimeoutError::Disconnected => Some(()),
            }),
            None => receiver.recv().map_err(|_| Some(())),
        };
        match command {
            Ok(TerminalOutputCommand::Data { data, input_traces }) => {
                pending_input_traces.extend(input_traces);
                let now = Instant::now();
                for ready in batcher.push(now, data) {
                    publish_terminal_output(
                        &context,
                        ready,
                        std::mem::take(&mut pending_input_traces),
                    );
                }
            }
            Ok(TerminalOutputCommand::Exit(exit_code)) => {
                if let Some(ready) = batcher.flush() {
                    publish_terminal_output(
                        &context,
                        ready,
                        std::mem::take(&mut pending_input_traces),
                    );
                }
                publish_terminal_exit(&context, exit_code);
                break;
            }
            Err(None) => {
                if let Some(ready) = batcher.flush_due(Instant::now()) {
                    publish_terminal_output(
                        &context,
                        ready,
                        std::mem::take(&mut pending_input_traces),
                    );
                }
            }
            Err(Some(())) => {
                if let Some(ready) = batcher.flush() {
                    publish_terminal_output(
                        &context,
                        ready,
                        std::mem::take(&mut pending_input_traces),
                    );
                }
                break;
            }
        }
    }
    let (drained, changed) = &*context.output_drained;
    *drained.lock() = true;
    changed.notify_all();
}

fn spawn_output_reader(mut output: NativePtyOutput, context: TerminalOutputReaderContext) {
    let (sender, receiver) = mpsc::sync_channel(OUTPUT_READER_QUEUE_CAPACITY);
    let first_provider_byte_started_at = context.first_provider_byte_started_at;
    let pending_input_traces = Arc::clone(&context.pending_input_traces);
    std::thread::spawn(move || run_output_processor(receiver, context));
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut pending = Vec::new();
        let mut first_provider_byte_recorded = false;
        loop {
            match output.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if !first_provider_byte_recorded {
                        first_provider_byte_recorded = true;
                        crate::other::telemetry::record_terminal_launch(
                            crate::other::telemetry::TerminalLaunch::FirstProviderByte,
                            first_provider_byte_started_at.elapsed(),
                        );
                    }
                    if let Some(filtered) = process_pty_output(&buf[..n], &mut pending) {
                        let input_traces =
                            pending_input_traces.lock().drain(..).collect::<Vec<_>>();
                        for trace in &input_traces {
                            crate::other::telemetry::record_terminal_input_output_read(trace);
                        }
                        if sender
                            .send(TerminalOutputCommand::Data {
                                data: filtered,
                                input_traces,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let exit_code = output.wait().ok().flatten();
        let _ = sender.send(TerminalOutputCommand::Exit(exit_code));
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
            input_ingress: Mutex::new(TerminalSurfaceInputIngressRegistry::default()),
            spawn_resolved: Condvar::new(),
            runtimes: Mutex::new(HashMap::new()),
            native_pty: NativePtySystem,
            journal_enabled: true,
            snapshot_materialization_count: std::sync::atomic::AtomicUsize::new(0),
            runtime: PhantomData,
        }
    }

    pub fn new_with_event_sink(
        app: AppHandle<R>,
        event_sink: Arc<dyn TerminalSurfaceEventSink>,
        journal_enabled: bool,
    ) -> Self {
        Self::new_with_event_sink_and_lifecycle_config(
            app,
            event_sink,
            journal_enabled,
            TerminalSurfaceLifecycleConfig::default(),
        )
    }

    pub(crate) fn new_with_event_sink_and_lifecycle_config(
        app: AppHandle<R>,
        event_sink: Arc<dyn TerminalSurfaceEventSink>,
        journal_enabled: bool,
        lifecycle_config: TerminalSurfaceLifecycleConfig,
    ) -> Self {
        Self {
            app: Some(app),
            event_sink: Some(event_sink),
            registry: Arc::new(Mutex::new(TerminalSurfaceRegistry::with_config(
                lifecycle_config,
            ))),
            input_ingress: Mutex::new(TerminalSurfaceInputIngressRegistry::default()),
            spawn_resolved: Condvar::new(),
            runtimes: Mutex::new(HashMap::new()),
            native_pty: NativePtySystem,
            journal_enabled,
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

    fn publish_input_unavailable(
        &self,
        session_key: &str,
        cause: TerminalSurfaceInputUnavailableCause,
    ) {
        if let Some(event_sink) = &self.event_sink {
            event_sink.publish(TerminalSurfaceEvent::InputUnavailable {
                session_key: session_key.to_string(),
                cause,
            });
        }
    }

    fn write_runtime(
        &self,
        session_key: &str,
        data: &str,
        input_trace: Option<crate::other::telemetry::TerminalInputTraceKey>,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let runtime_generation = {
            let registry = self.registry.lock();
            let surface = registry.find_by_session_key(session_key).ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!(
                    "Terminal Surface not found for owner {session_key}"
                ))
            })?;
            if surface.ensure_writable().is_err() {
                return Err(TerminalSurfaceGatewayError::new(format!(
                    "Terminal Surface is not writable for owner {session_key}"
                )));
            }
            surface.runtime_generation.value()
        };
        let runtimes = self.runtimes.lock();
        let runtime = runtimes.get(&runtime_generation).ok_or_else(|| {
            TerminalSurfaceGatewayError::new(format!("PTY {} not found", runtime_generation))
        })?;
        if let Some(trace) = &input_trace {
            crate::other::telemetry::record_terminal_input_writer_enqueue(
                trace.attachment_id(),
                trace.sequence(),
            );
            runtime.pending_input_traces.lock().push_back(trace.clone());
        }
        if let Err(error) = runtime.native_pty.write(data.as_bytes()) {
            if let Some(trace) = &input_trace {
                runtime
                    .pending_input_traces
                    .lock()
                    .retain(|pending| pending != trace);
            }
            return Err(TerminalSurfaceGatewayError::new(error));
        }
        Ok(())
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
        TerminalCheckpointFileStore::new(&data_dir, TERMINAL_SURFACE_SCROLLBACK_ROWS)
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
        TerminalCheckpointFileStore::new(&data_dir, TERMINAL_SURFACE_SCROLLBACK_ROWS)
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
        let integration_dir = if request.process.is_some() {
            None
        } else {
            crate::infrastructure::terminal::shell_integration::create_shell_integration_files(
                &app_data_dir,
            )
            .ok()
        };
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        let mut extra_env = Vec::new();
        let child_environment = Instant::now();
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
        crate::other::telemetry::record_terminal_launch(
            crate::other::telemetry::TerminalLaunch::ChildEnvironment,
            child_environment.elapsed(),
        );
        let pty_open_and_spawn = Instant::now();
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
                process: request.process.map(|process| NativePtyProcessConfig {
                    executable: process.executable().to_os_string(),
                    arguments: process.arguments().to_vec(),
                    environment: process.environment().to_vec(),
                }),
            })
            .map_err(TerminalSurfaceGatewayError::new)?;
        crate::other::telemetry::record_terminal_launch(
            crate::other::telemetry::TerminalLaunch::PtyOpenAndSpawn,
            pty_open_and_spawn.elapsed(),
        );

        let initial_checkpoint =
            request
                .initial_terminal_surface
                .as_ref()
                .map(|checkpoint| NativeTerminalCheckpoint {
                    replay: checkpoint.replay.clone(),
                    sequence: checkpoint.sequence,
                    cols: checkpoint.cols,
                    rows: checkpoint.rows,
                });
        let terminal_surface = initial_checkpoint.as_ref().map_or_else(
            || {
                NativeTerminalEmulator::new(
                    request.cols,
                    request.rows,
                    TERMINAL_SURFACE_SCROLLBACK_ROWS,
                )
            },
            |checkpoint| {
                NativeTerminalEmulator::restore(checkpoint, TERMINAL_SURFACE_SCROLLBACK_ROWS)
            },
        );
        let checkpoint_store =
            TerminalCheckpointFileStore::new(&app_data_dir, TERMINAL_SURFACE_SCROLLBACK_ROWS);
        let terminal_surface = Arc::new(Mutex::new(terminal_surface));
        let checkpoint_journal = Arc::new(Mutex::new(IncrementalCheckpointJournal::new(
            initial_checkpoint
                .clone()
                .unwrap_or(NativeTerminalCheckpoint {
                    replay: String::new(),
                    sequence: 0,
                    cols: request.cols,
                    rows: request.rows,
                }),
            initial_checkpoint.is_some(),
        )));
        let checkpoint_io = Arc::new(Mutex::new(()));
        let checkpoint_scheduler = Some({
            let store = checkpoint_store.clone();
            let registry = Arc::clone(&self.registry);
            let terminal_surface = Arc::clone(&terminal_surface);
            let checkpoint_journal = Arc::clone(&checkpoint_journal);
            let checkpoint_io = Arc::clone(&checkpoint_io);
            let session_key = request.session_key.clone();
            DirtyCheckpointScheduler::spawn(
                CHECKPOINT_PERSIST_INTERVAL,
                Arc::new(move || {
                    let _io = checkpoint_io.lock();
                    flush_incremental_checkpoint(
                        &store,
                        &session_key,
                        &registry,
                        runtime_generation,
                        &terminal_surface,
                        &checkpoint_journal,
                    )
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
            checkpoint_journal: Some(checkpoint_journal),
            checkpoint_store: Some(checkpoint_store),
            checkpoint_io: Some(checkpoint_io),
            pending_input_traces: Arc::new(Mutex::new(VecDeque::new())),
        };
        let mut runtimes = self.runtimes.lock();
        if runtimes.contains_key(&runtime_generation) {
            return Err(TerminalSurfaceGatewayError::new(format!(
                "PTY {runtime_generation} already exists"
            )));
        }
        runtimes.insert(runtime_generation, runtime);
        if let Some(checkpoint_scheduler) = runtimes
            .get(&runtime_generation)
            .and_then(|runtime| runtime.checkpoint_scheduler.clone())
        {
            checkpoint_scheduler.mark_dirty();
        }
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
            checkpoint_journal,
            pending_input_traces,
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
                runtime.checkpoint_journal.clone(),
                Arc::clone(&runtime.pending_input_traces),
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
                checkpoint_journal,
                journal_enabled: self.journal_enabled,
                first_provider_byte_started_at: Instant::now(),
                pending_input_traces,
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

    fn activate_input_attachment(&self, session_key: &str, attachment_id: &str) {
        self.input_ingress
            .lock()
            .activate(session_key, attachment_id);
    }

    fn deactivate_input_attachment(&self, session_key: &str, attachment_id: &str) {
        self.input_ingress
            .lock()
            .deactivate(session_key, attachment_id);
    }

    fn write_attached(
        &self,
        session_key: &str,
        attachment_id: &str,
        sequence: u64,
        data: &str,
    ) -> Result<(), TerminalSurfaceGatewayError> {
        let mut ingress = self.input_ingress.lock();
        let mut ready = match ingress.admit(session_key, attachment_id, sequence, data.to_string())
        {
            Ok(ready) => ready,
            Err(TerminalSurfaceInputIngressError::StaleAttachment) => {
                let cause = TerminalSurfaceInputUnavailableCause::StaleAttachment;
                let error = TerminalSurfaceGatewayError::new(cause.internal_cause());
                drop(ingress);
                self.publish_input_unavailable(session_key, cause);
                return Err(error);
            }
            Err(TerminalSurfaceInputIngressError::PendingCapacityExceeded) => {
                let cause = TerminalSurfaceInputUnavailableCause::PendingCapacityExceeded;
                let error = TerminalSurfaceGatewayError::new(cause.internal_cause());
                let should_publish = ingress.record_failure(session_key, attachment_id);
                drop(ingress);
                if should_publish {
                    self.publish_input_unavailable(session_key, cause);
                }
                return Err(error);
            }
        };
        for index in 0..ready.len() {
            let result = self.write_runtime(
                session_key,
                &ready[index].data,
                crate::other::telemetry::terminal_input_trace_key(
                    attachment_id,
                    ready[index].sequence,
                ),
            );
            if let Err(error) = result {
                let failed = ready.split_off(index);
                let _ = ingress.restore_failed(session_key, attachment_id, failed);
                let should_publish = ingress.record_failure(session_key, attachment_id);
                drop(ingress);
                if should_publish {
                    self.publish_input_unavailable(
                        session_key,
                        TerminalSurfaceInputUnavailableCause::RuntimeWriteFailed(
                            error.message().to_string(),
                        ),
                    );
                }
                return Err(error);
            }
        }
        if !ready.is_empty() {
            ingress.record_success(session_key, attachment_id);
        }
        Ok(())
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
        self.write_runtime(session_key, data, None)
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
        let (
            native_pty,
            event_order,
            terminal_surface,
            checkpoint_scheduler,
            checkpoint_journal,
            session_key,
        ) = {
            let runtimes = self.runtimes.lock();
            let runtime = runtimes.get(&runtime_generation).ok_or_else(|| {
                TerminalSurfaceGatewayError::new(format!("PTY {} not found", runtime_generation))
            })?;
            (
                runtime.native_pty.clone(),
                Arc::clone(&runtime.event_order),
                Arc::clone(&runtime.terminal_surface),
                runtime.checkpoint_scheduler.clone(),
                runtime.checkpoint_journal.clone(),
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
            if let Some(journal) = checkpoint_journal {
                journal
                    .lock()
                    .record(NativeTerminalCheckpointRecord::Resize {
                        sequence,
                        cols,
                        rows,
                    })
                    .map_err(TerminalSurfaceGatewayError::new)?;
            }
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
        let targets = self
            .runtimes
            .lock()
            .values()
            .filter_map(|runtime| {
                Some((
                    runtime.checkpoint_scheduler.clone()?,
                    runtime.checkpoint_store.clone()?,
                    runtime.checkpoint_journal.clone()?,
                    runtime.checkpoint_io.clone()?,
                    Arc::clone(&runtime.terminal_surface),
                    runtime.session_key.clone(),
                ))
            })
            .collect::<Vec<_>>();
        for (scheduler, _, _, _, _, _) in &targets {
            scheduler
                .flush()
                .map_err(TerminalSurfaceGatewayError::new)?;
        }
        for (_, store, journal, checkpoint_io, terminal_surface, session_key) in targets {
            let runtime_generation = self
                .runtime_generation_for_session_key(&session_key)
                .ok_or_else(|| {
                    TerminalSurfaceGatewayError::new(format!(
                        "Terminal Surface not found for owner {session_key}"
                    ))
                })?;
            let _io = checkpoint_io.lock();
            compact_checkpoint(
                &store,
                &session_key,
                &self.registry,
                runtime_generation,
                &terminal_surface,
                &journal,
            )
            .map_err(TerminalSurfaceGatewayError::new)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "runtime_gateway_impl_test.rs"]
mod runtime_gateway_impl_tests;
