use super::*;

pub fn insert_runtime(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
    session_key: String,
    native_pty: NativePtyRuntime,
    terminal_surface: Arc<Mutex<NativeTerminalEmulator>>,
) {
    gateway.runtimes.lock().insert(
        runtime_generation,
        AttachedTerminalRuntime {
            native_pty,
            output: None,
            event_order: Arc::new(TerminalSurfaceEventOrder::default()),
            terminal_surface,
            checkpoint_scheduler: None,
            session_key,
            output_drained: Arc::new((Mutex::new(true), Condvar::new())),
            checkpoint_journal: None,
            checkpoint_store: None,
            checkpoint_io: None,
        },
    );
}

pub fn attach_checkpoint(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
    store: TerminalCheckpointFileStore,
    journal: Arc<Mutex<IncrementalCheckpointJournal>>,
    background_flush: bool,
) {
    let io = Arc::new(Mutex::new(()));
    let mut runtimes = gateway.runtimes.lock();
    let runtime = runtimes.get_mut(&runtime_generation).unwrap();
    let background = Arc::new(BackgroundCheckpoint {
        store: store.clone(),
        session_key: runtime.session_key.clone(),
        registry: gateway.registry.clone(),
        runtime_generation,
        terminal_surface: runtime.terminal_surface.clone(),
        journal: journal.clone(),
        io: io.clone(),
    });
    let flush = background.clone();
    runtime.checkpoint_scheduler = Some(CheckpointScheduler {
        dirty: Arc::new(|_| {}),
        session_key: runtime.session_key.clone(),
        flush: Arc::new(move || {
            if background_flush {
                futures_executor::block_on(flush.flush())
            } else {
                Ok(())
            }
        }),
        background,
    });
    runtime.checkpoint_store = Some(store);
    runtime.checkpoint_journal = Some(journal);
    runtime.checkpoint_io = Some(io);
}

pub fn remove_checkpoint_target(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
) {
    gateway.registry.lock().remove(runtime_generation);
}

pub fn compact_runtime_checkpoint(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
) -> Result<(), TerminalSurfaceGatewayError> {
    let runtimes = gateway.runtimes.lock();
    let runtime = runtimes.get(&runtime_generation).unwrap();
    compact_checkpoint(
        runtime.checkpoint_store.as_ref().unwrap(),
        &runtime.session_key,
        &gateway.registry,
        runtime_generation,
        &runtime.terminal_surface,
        runtime.checkpoint_journal.as_ref().unwrap(),
    )
    .map_err(checkpoint_work_failure)
}

pub fn replace_checkpoint_flush(
    gateway: &TerminalSurfaceRuntimeGatewayFor,
    runtime_generation: u64,
    flush: Arc<dyn Fn() -> Result<(), WorkFailure> + Send + Sync>,
) {
    gateway
        .runtimes
        .lock()
        .get_mut(&runtime_generation)
        .unwrap()
        .checkpoint_scheduler
        .as_mut()
        .unwrap()
        .flush = flush;
}

#[derive(Clone)]
pub struct BackgroundCheckpointFixture(Arc<BackgroundCheckpoint>);

impl BackgroundCheckpointFixture {
    pub fn new(
        store: TerminalCheckpointFileStore,
        session_key: String,
        runtime_generation: u64,
        terminal_surface: Arc<Mutex<NativeTerminalEmulator>>,
        journal: Arc<Mutex<IncrementalCheckpointJournal>>,
    ) -> Self {
        Self(Arc::new(BackgroundCheckpoint {
            store,
            session_key,
            registry: Arc::new(Mutex::new(TerminalSurfaceRegistry::default())),
            runtime_generation,
            terminal_surface,
            journal,
            io: Arc::new(Mutex::new(())),
        }))
    }

    pub fn io_available(&self) -> bool {
        self.0.io.try_lock().is_some()
    }

    pub async fn flush(&self) -> Result<(), WorkFailure> {
        self.0.flush().await
    }
}
