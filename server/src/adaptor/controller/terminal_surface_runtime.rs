use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "test-support")]
use crate::adaptor::presenter::terminal::{
    GetOrSpawnTerminalV1, TerminalProcessLaunchV1, TerminalSurfaceOwnerV1, TerminalSurfaceV1,
};
use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;

pub struct TerminalSurfaceRuntime {
    application: Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
}

pub struct BackgroundWork {
    pub retrying: Arc<crate::usecase::retry::Retrying>,
    #[cfg(feature = "test-support")]
    pub failures: Arc<crate::adaptor::gateway::failure_records::FailureRecordStore>,
    pub handle: tokio::runtime::Handle,
}

impl BackgroundWork {
    pub(crate) fn new(
        retrying: Arc<crate::usecase::retry::Retrying>,
        _failures: Arc<crate::adaptor::gateway::failure_records::FailureRecordStore>,
        handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            retrying,
            #[cfg(feature = "test-support")]
            failures: _failures,
            handle,
        }
    }
}

#[cfg(feature = "test-support")]
pub use crate::adaptor::presenter::terminal_event_fault_relay::TerminalSurfaceEventFault;
#[cfg(feature = "test-support")]
pub use crate::adaptor::presenter::terminal_event_fault_relay::TerminalSurfaceEventFaultController;

impl TerminalSurfaceRuntime {
    pub fn new(work: Arc<BackgroundWork>, data_dir: PathBuf) -> Self {
        Self::compose(work, data_dir)
    }

    #[doc(hidden)]
    #[cfg(feature = "test-support")]
    pub fn new_with_data_dir_and_event_faults(
        work: Arc<BackgroundWork>,
        data_dir: PathBuf,
    ) -> (Self, TerminalSurfaceEventFaultController) {
        let event_hub =
            Arc::new(crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new());
        let event_target: Arc<dyn TerminalSurfaceEventSink> = event_hub.clone();
        let (event_sink, faults) =
            crate::adaptor::presenter::terminal_event_fault_relay::fault_injecting_event_sink(
                event_target,
            );
        (
            Self::compose_with_event_transport(work, data_dir, event_hub, event_sink),
            faults,
        )
    }

    fn compose(work: Arc<BackgroundWork>, data_dir: PathBuf) -> Self {
        let event_hub =
            Arc::new(crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new());
        let event_sink: Arc<dyn TerminalSurfaceEventSink> = event_hub.clone();
        Self::compose_with_event_transport(work, data_dir, event_hub, event_sink)
    }

    fn compose_with_event_transport(
        work: Arc<BackgroundWork>,
        data_dir: PathBuf,
        event_hub: Arc<crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub>,
        event_sink: Arc<dyn TerminalSurfaceEventSink>,
    ) -> Self {
        let journal_enabled = true;
        let (dirty, dirty_receiver) = super::terminal_checkpoint::dirty_channel();
        let gateway = Arc::new(crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(dirty,
            data_dir,
            event_sink,
            journal_enabled,
        ));
        let application = Arc::new(
            crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
                std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
                gateway,
                Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(event_hub.event_sender())),
                event_hub,
            ),
        );
        let terminal = application.clone();
        work.handle.spawn(super::terminal_checkpoint::run(
            work.retrying.clone(),
            move |session_key| {
                let terminal = terminal.clone();
                async move { terminal.flush_checkpoint(&session_key).await }
            },
            dirty_receiver,
            crate::infrastructure::timer::delays(
                super::terminal_checkpoint::CHECKPOINT_PERSIST_INTERVAL,
            ),
        ));
        Self { application }
    }

    pub(crate) fn application(
        &self,
    ) -> Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication> {
        Arc::clone(&self.application)
    }

    #[cfg(feature = "test-support")]
    pub fn get_or_spawn(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwnerV1,
        label: Option<String>,
    ) -> Result<GetOrSpawnTerminalV1, String> {
        self.application
            .get_or_spawn(rows, cols, cwd, owner.try_into()?, label, None)
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(feature = "test-support")]
    pub fn get_or_spawn_with_startup(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwnerV1,
        label: Option<String>,
        startup_command: Option<String>,
    ) -> Result<GetOrSpawnTerminalV1, String> {
        self.application
            .get_or_spawn(rows, cols, cwd, owner.try_into()?, label, startup_command)
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(feature = "test-support")]
    pub fn get_or_spawn_with_process(
        &self,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        owner: TerminalSurfaceOwnerV1,
        label: Option<String>,
        process: TerminalProcessLaunchV1,
    ) -> Result<GetOrSpawnTerminalV1, String> {
        self.application
            .get_or_spawn_process(
                rows,
                cols,
                cwd,
                owner.try_into()?,
                label,
                process.try_into()?,
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "test-support")]
    pub fn get(&self, owner: TerminalSurfaceOwnerV1) -> Result<TerminalSurfaceV1, String> {
        self.application
            .get(&owner.try_into()?)
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "test-support")]
    pub fn write(&self, owner: TerminalSurfaceOwnerV1, data: &str) -> Result<(), String> {
        self.application
            .write(&owner.try_into()?, data)
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "test-support")]
    pub fn resize(
        &self,
        owner: TerminalSurfaceOwnerV1,
        rows: u16,
        cols: u16,
    ) -> Result<(), String> {
        self.application
            .resize(&owner.try_into()?, rows, cols)
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "test-support")]
    pub fn kill(&self, owner: TerminalSurfaceOwnerV1) -> Result<(), String> {
        self.application
            .kill(&owner.try_into()?)
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "test-support")]
    pub fn flush_checkpoints(&self) -> Result<(), String> {
        self.application
            .flush_checkpoints()
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "test-support")]
    pub fn shutdown(&self) -> Result<(), String> {
        self.application
            .shutdown()
            .map_err(|error| error.to_string())
    }
}

#[doc(hidden)]
#[cfg(feature = "test-support")]
pub fn initialize_background_work_for_acceptance() -> Arc<BackgroundWork> {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("acceptance background runtime")
    });
    let _entered = runtime.enter();
    let failures =
        Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default());
    Arc::new(BackgroundWork::new(
        crate::usecase::retry::Retrying::new(
            Arc::new(crate::common::retry::RetryLimiter::new()),
            Arc::new(crate::usecase::failure::FailureRecordingUsecase::new(
                failures.clone(),
                None,
            )),
        ),
        failures,
        runtime.handle().clone(),
    ))
}
