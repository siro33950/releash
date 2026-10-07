use crate::adaptor::controller::command::CommandRouter;
use releashd::desktop_api::ClientEndpoint;
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
    let (settings_sources, settings_updates) = tokio::sync::watch::channel(None);
    let connection = Arc::new(
        crate::adaptor::gateway::daemon_connection::DaemonServiceGateway::new(
            executable.into(),
            data_dir.into(),
            Arc::new(crate::common::retry::RetryLimiter::new()),
            settings_sources,
        ),
    );
    let preference = Arc::new(crate::adaptor::gateway::login_item::DaemonLoginPreference(
        connection.clone(),
    ));
    let connection = Arc::new(
        crate::usecase::daemon_connection::DaemonConnectionUsecase::new(
            connection.clone(),
            connection,
        ),
    );
    let connecting = connection.clone();
    tauri::async_runtime::spawn(async move {
        let _ = connecting.connect().await;
    });
    let login = Arc::new(RecordingLoginItem::default());
    let login_usecase = Arc::new(crate::usecase::login_item::LoginItemUsecase::new(
        login.clone(),
        preference,
    ));
    let app = builder
        .manage(connection.clone())
        .manage(login.clone())
        .manage(login_usecase.clone())
        .invoke_handler(move |invoke| router.handle(invoke))
        .build(crate::application_context())
        .unwrap();
    app.manage(Arc::new(
        crate::usecase::desktop_lifecycle::DesktopLifecycleUsecase::new(
            connection,
            login_usecase,
            Arc::new(
                crate::adaptor::gateway::desktop_lifecycle::TauriDesktopLifecycle(Arc::new(
                    crate::infrastructure::platform::desktop_runtime::DesktopRuntime::new(
                        app.handle().clone(),
                    ),
                )),
            ),
        ),
    ));
    let observer_app = app.handle().clone();
    tauri::async_runtime::spawn(crate::infrastructure::settings_observer::observe(
        settings_updates,
        move |settings| {
            crate::adaptor::controller::desktop_lifecycle::settings_changed(
                &observer_app,
                settings,
            );
        },
    ));
    app
}

#[derive(Default)]
struct RecordingLoginItem {
    registered: std::sync::atomic::AtomicBool,
    calls: parking_lot::Mutex<Vec<&'static str>>,
}
impl crate::domain::login_item::LoginItemPort for RecordingLoginItem {
    fn status(&self) -> Result<crate::domain::login_item::LoginItemStatus, String> {
        self.calls.lock().push("status");
        Ok(
            if self.registered.load(std::sync::atomic::Ordering::SeqCst) {
                crate::domain::login_item::LoginItemStatus::Enabled
            } else {
                crate::domain::login_item::LoginItemStatus::NotRegistered
            },
        )
    }
    fn location(&self) -> Result<crate::domain::login_item::RegistrationLocation, String> {
        Ok(crate::domain::login_item::RegistrationLocation {
            translocated: false,
            read_only: false,
        })
    }
    fn register(&self) -> Result<(), String> {
        self.calls.lock().push("register");
        self.registered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn unregister(&self) -> Result<(), String> {
        self.calls.lock().push("unregister");
        self.registered
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn open_settings(&self) -> Result<(), String> {
        Ok(())
    }
}

pub async fn initialize_desktop<R: tauri::Runtime>(app: &tauri::AppHandle<R>, hidden: bool) {
    crate::adaptor::controller::desktop_lifecycle::initialize(app, hidden).await;
}
pub fn desktop_login_item_calls<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Vec<&'static str> {
    app.state::<Arc<RecordingLoginItem>>().calls.lock().clone()
}
pub async fn desktop_login_preference<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    requested: Option<bool>,
) -> bool {
    let login = app.state::<Arc<crate::usecase::login_item::LoginItemUsecase>>();
    if let Some(requested) = requested {
        login.set_enabled(requested).await.unwrap().requested
    } else {
        login.status().await.unwrap().requested
    }
}

pub async fn initialize_desktop_settings<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let settings = app
        .state::<Arc<crate::usecase::daemon_connection::DaemonConnectionUsecase>>()
        .settings()
        .unwrap();
    crate::adaptor::gateway::desktop_lifecycle::apply_desktop_settings(app, settings);
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

pub async fn desktop_client_endpoint<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> ClientEndpoint {
    let endpoint = crate::adaptor::controller::command::client::get_client_endpoint(app.state())
        .await
        .unwrap();
    ClientEndpoint {
        url: endpoint.url,
        token: endpoint.token,
    }
}
pub type DesktopUpdateAction = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub async fn apply_desktop_update<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
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
    impl crate::usecase::desktop_update::DesktopUpdateInstaller for Installer {
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
    crate::usecase::desktop_update::DesktopUpdateUsecase::new(Arc::new(Installer(action)))
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

pub fn desktop_connection_status<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> serde_json::Value {
    let connection = app.state::<Arc<crate::usecase::daemon_connection::DaemonConnectionUsecase>>();
    if let Some(failure) = connection
        .failure()
        .map(crate::adaptor::presenter::daemon_connection::failure)
    {
        serde_json::json!({"phase":"failed", "reason":failure.message})
    } else {
        serde_json::json!({"phase":if connection.settings().is_some() {"ready"} else {"starting"}})
    }
}
pub async fn start_desktop_daemon<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    app.state::<Arc<crate::usecase::daemon_connection::DaemonConnectionUsecase>>()
        .connect()
        .await
        .unwrap();
}
pub async fn stop_desktop_daemon<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    app.state::<Arc<crate::usecase::daemon_connection::DaemonConnectionUsecase>>()
        .stop()
        .await
        .unwrap();
}

#[cfg(target_os = "macos")]
pub fn install_desktop_native_quit(app: &tauri::AppHandle) {
    let handle = app.clone();
    crate::infrastructure::platform::native_termination::install(move || {
        crate::adaptor::controller::desktop_lifecycle::quit(&handle, 0)
    })
    .unwrap();
}
#[cfg(target_os = "macos")]
pub fn dispatch_desktop_tray_quit(app: &tauri::AppHandle) {
    crate::infrastructure::platform::tray::dispatch_menu_event(
        crate::infrastructure::platform::tray::ids::QUIT,
        || panic!("unexpected show"),
        || crate::adaptor::controller::desktop_lifecycle::quit(app, 0),
        || panic!("unexpected stop"),
    );
}

#[cfg(target_os = "macos")]
pub fn quit_desktop(app: tauri::AppHandle) {
    crate::adaptor::controller::command::desktop_lifecycle::quit_desktop(app);
}
