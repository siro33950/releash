use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::protocol::terminal::{
    GetOrSpawnTerminalV1, TerminalProcessLaunchV1, TerminalSurfaceOwnerV1,
    TerminalSurfaceStreamItemV1, TerminalSurfaceV1,
};
use crate::domain::terminal_surface::gateway::TerminalSurfaceEventSink;

pub struct TerminalSurfaceRuntime {
    application: Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    activity_tap: Arc<
        crate::adaptor::controller::agent_session_activity_observer::AgentSessionActivityEventTap,
    >,
}

pub struct TerminalSurfaceWireAttachment {
    receiver: tokio::sync::mpsc::Receiver<TerminalSurfaceStreamItemV1>,
}

pub use crate::adaptor::gateway::terminal_surface::event_fault_relay::{
    TerminalSurfaceEventFault, TerminalSurfaceEventFaultController,
};

impl TerminalSurfaceWireAttachment {
    pub async fn next(&mut self) -> Option<TerminalSurfaceStreamItemV1> {
        self.receiver.recv().await
    }
}

impl TerminalSurfaceRuntime {
    pub fn new<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Self {
        Self::compose(app)
    }

    pub fn new_with_data_dir<R: tauri::Runtime>(
        app: tauri::AppHandle<R>,
        data_dir: PathBuf,
    ) -> Self {
        if app
            .try_state::<crate::infrastructure::platform::app_data_dir::TestDataDir>()
            .is_none()
        {
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                data_dir,
            ));
        }
        Self::compose(app)
    }

    #[doc(hidden)]
    pub fn new_with_data_dir_and_event_faults<R: tauri::Runtime>(
        app: tauri::AppHandle<R>,
        data_dir: PathBuf,
    ) -> (Self, TerminalSurfaceEventFaultController) {
        if app
            .try_state::<crate::infrastructure::platform::app_data_dir::TestDataDir>()
            .is_none()
        {
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                data_dir,
            ));
        }
        let event_hub = Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        );
        let event_target: Arc<dyn TerminalSurfaceEventSink> = event_hub.clone();
        let (event_sink, faults) = crate::adaptor::gateway::terminal_surface::event_fault_relay::fault_injecting_event_sink(event_target);
        (
            Self::compose_with_event_transport(app, event_hub, event_sink),
            faults,
        )
    }

    fn compose<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Self {
        let event_hub = Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        );
        let event_sink: Arc<dyn TerminalSurfaceEventSink> = event_hub.clone();
        Self::compose_with_event_transport(app, event_hub, event_sink)
    }

    fn compose_with_event_transport<R: tauri::Runtime>(
        app: tauri::AppHandle<R>,
        event_hub: Arc<
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub,
        >,
        event_sink: Arc<dyn TerminalSurfaceEventSink>,
    ) -> Self {
        let journal_enabled = !crate::other::performance_switches::terminal_performance_switches()
            .disable_terminal_journal;
        let activity_tap = Arc::new(
            crate::adaptor::controller::agent_session_activity_observer::AgentSessionActivityEventTap::new(
                event_sink,
            ),
        );
        let gateway = Arc::new(
            crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
                app,
                activity_tap.clone(),
                journal_enabled,
            ),
        );
        Self {
            application: Arc::new(
                crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
                    gateway, event_hub,
                ),
            ),
            activity_tap,
        }
    }

    pub(crate) fn application(
        &self,
    ) -> Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication> {
        Arc::clone(&self.application)
    }

    /// AgentSession activity usecase を terminal event tap へ後結合する。
    /// terminal 側 composition が provider AgentSession composition より先に
    /// 完了するため、bind でサイクルを断つ。
    pub(crate) fn bind_agent_session_activity(
        &self,
        activity: Arc<crate::usecase::agent_session::AgentSessionActivityUsecase>,
    ) {
        self.activity_tap.bind(activity);
    }

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

    pub fn get(&self, owner: TerminalSurfaceOwnerV1) -> Result<TerminalSurfaceV1, String> {
        self.application
            .get(&owner.try_into()?)
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    pub fn write(&self, owner: TerminalSurfaceOwnerV1, data: &str) -> Result<(), String> {
        self.application
            .write(&owner.try_into()?, data)
            .map_err(|error| error.to_string())
    }

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

    pub fn kill(&self, owner: TerminalSurfaceOwnerV1) -> Result<(), String> {
        self.application
            .kill(&owner.try_into()?)
            .map_err(|error| error.to_string())
    }

    pub fn flush_checkpoints(&self) -> Result<(), String> {
        self.application
            .flush_checkpoints()
            .map_err(|error| error.to_string())
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.application
            .shutdown()
            .map_err(|error| error.to_string())
    }

    pub fn attach(
        &self,
        attachment_id: String,
        owner: TerminalSurfaceOwnerV1,
    ) -> Result<TerminalSurfaceWireAttachment, String> {
        let attachment = self
            .application
            .attach(&attachment_id, &owner.try_into()?)
            .map_err(|error| error.to_string())?;
        let (sender, receiver) = tokio::sync::mpsc::channel(256);
        tauri::async_runtime::spawn(
            crate::adaptor::controller::command::terminal_surface::commands::forward_terminal_surface_attachment(
                Arc::clone(&self.application),
                attachment_id,
                attachment,
                move |item| sender.try_send(item).map_err(|error| error.to_string()),
            ),
        );
        Ok(TerminalSurfaceWireAttachment { receiver })
    }
}
