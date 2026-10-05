use crate::adaptor::controller::command::CommandRouter;
use crate::client_api_acceptance::ClientEndpoint;
use std::path::Path;
use std::sync::Arc;
use tauri::Manager;

pub fn desktop_connection_app<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    data_dir: &Path,
    executable: &Path,
) -> tauri::App<R> {
    let mut router: CommandRouter<Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>> =
        CommandRouter::new(Box::new(|_| false));
    crate::adaptor::controller::command::client::register(&mut router);
    crate::adaptor::controller::command::desktop_lifecycle::register(&mut router);
    let status_presenter =
        Arc::new(crate::adaptor::presenter::daemon_status::DaemonStatusPresenter::new());
    let supervisor = crate::usecase::daemon_supervision::DaemonSupervisionUsecase::start(
        Arc::new(
            crate::adaptor::gateway::daemon_supervision::DaemonProcessGateway::new(
                executable.into(),
                data_dir.into(),
                Arc::new(crate::common::retry::RetryLimiter::new()),
            ),
        ),
        status_presenter.clone(),
    );
    builder
        .manage(status_presenter)
        .manage(supervisor)
        .invoke_handler(move |invoke| router.handle(invoke))
        .build(crate::application_context())
        .unwrap()
}

pub fn desktop_supervision_status<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> serde_json::Value {
    serde_json::to_value(
        crate::adaptor::presenter::daemon_status::DaemonStatusMessage::from(
            app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
                .status(),
        ),
    )
    .unwrap()
}

pub fn stop_desktop_daemon<R: tauri::Runtime>(app: &tauri::AppHandle<R>, restart: bool) {
    use crate::domain::daemon_supervision::StopIntent;
    app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
        .stop(if restart {
            StopIntent::Restart
        } else {
            StopIntent::Quit(0)
        })
        .unwrap();
}

pub async fn initialize_desktop_settings<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let settings = app
        .state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
        .connection()
        .unwrap()
        .settings;
    crate::desktop::apply_desktop_settings(app, settings);
}

pub fn desktop_window_preferences<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use tauri::Manager;
    let settings = app
        .state::<crate::infrastructure::platform::window_lifecycle::WindowPreferencesState>()
        .read();
    settings.close_to_tray
}

pub fn spawn_desktop_successor() -> Result<(), String> {
    crate::infrastructure::platform::desktop_restart::spawn_successor()
}

pub fn wait_for_desktop_predecessor() -> Result<bool, String> {
    crate::infrastructure::platform::desktop_restart::wait_for_predecessor()
}

pub async fn terminate_daemon_for_acceptance(
    executable: std::path::PathBuf,
    data_dir: std::path::PathBuf,
) -> Result<std::time::Duration, String> {
    use crate::domain::daemon_supervision::DaemonProcessPort;
    let gateway = crate::adaptor::gateway::daemon_supervision::DaemonProcessGateway::new(
        executable,
        data_dir.clone(),
        Arc::new(crate::common::retry::RetryLimiter::new()),
    );
    gateway.spawn().await?;
    let ready = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !data_dir.join("ready").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    let duplicate_rejected = gateway.spawn().await.is_err();
    let start = std::time::Instant::now();
    gateway.terminate_and_wait().await?;
    ready.map_err(|e| e.to_string())?;
    if !duplicate_rejected {
        return Err("A second daemon was spawned before the previous one exited".into());
    }
    Ok(start.elapsed())
}

pub async fn desktop_client_endpoint<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> ClientEndpoint {
    let supervisor =
        app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>();
    let connection = supervisor.attach().await.unwrap();
    ClientEndpoint {
        url: connection.endpoint.url,
        token: connection.endpoint.token,
        launch_id: connection.launch_id,
    }
}

#[cfg(feature = "performance")]
pub fn probe_cli_installation() -> Result<String, String> {
    crate::infrastructure::platform::cli_install::install_cli()
}

pub type DesktopUpdateAction = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub async fn apply_desktop_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: DesktopUpdateAction,
) -> Result<(), String> {
    struct Installer(DesktopUpdateAction);
    #[async_trait::async_trait]
    impl crate::usecase::desktop_update::DesktopUpdateGateway for Installer {
        async fn check(
            &self,
        ) -> Result<Option<crate::usecase::desktop_update::UpdateInfo>, String> {
            Ok(None)
        }
    }
    #[async_trait::async_trait]
    impl crate::domain::daemon_supervision::DesktopUpdateInstaller for Installer {
        async fn download(&self) -> Result<(), String> {
            (self.0)("download")
        }
        async fn install(&self) -> Result<(), String> {
            (self.0)("install")
        }
        fn restart(&self) -> Result<(), String> {
            (self.0)("restart")
        }
    }
    crate::usecase::desktop_update::DesktopUpdateUsecase::new(
        Arc::new(Installer(action)),
        app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
            .inner()
            .clone(),
    )
    .apply()
    .await
    .map_err(|e| e.to_string())
}

pub fn command_rejection_app<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::App<R> {
    let router: CommandRouter<Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>> =
        CommandRouter::new(Box::new(|_| false));
    builder
        .invoke_handler(move |invoke| router.handle(invoke))
        .build(crate::application_context())
        .unwrap()
}
