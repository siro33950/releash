use crate::{adaptor, infrastructure, other, usecase};
use std::sync::Arc;
use tauri::Manager;

pub(crate) fn application_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

pub(crate) fn configure_client_connection<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    data_dir: std::path::PathBuf,
) -> Result<(), usecase::client_connection::ClientConnectionError> {
    let connection = usecase::client_connection::ClientConnectionUsecase(Box::new(
        adaptor::gateway::local_api::ClientConnectionFileQuery(data_dir),
    ));
    connection.endpoint()?;
    app.manage(connection);
    Ok(())
}

pub(crate) fn apply_desktop_settings<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: usecase::app_config::query_service::DesktopSettingsDto,
) {
    use usecase::telemetry::TelemetryPort;
    let preferences = infrastructure::platform::window_lifecycle::WindowPreferences {
        start_minimized: settings.start_minimized,
        close_to_tray: settings.close_to_tray,
    };
    if let Some(state) =
        app.try_state::<infrastructure::platform::window_lifecycle::WindowPreferencesState>()
    {
        *state.0.write() = preferences;
    } else {
        app.manage(
            infrastructure::platform::window_lifecycle::WindowPreferencesState(
                parking_lot::RwLock::new(preferences),
            ),
        );
    }
    adaptor::gateway::telemetry::TelemetryGateway
        .set_crash_reporting_enabled(settings.crash_reporting);
    adaptor::gateway::telemetry::TelemetryGateway
        .set_performance_enabled(settings.performance_telemetry);
}

pub fn run() {
    other::telemetry::set_startup_origin(std::time::Instant::now());
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ));
    #[cfg(feature = "performance-wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());
    let builder = builder.setup(|app| {
        let data_dir = infrastructure::platform::app_data_dir::resolve_data_dir()?;
        if let Err(error) = infrastructure::local_log::init(
            &data_dir,
            infrastructure::local_log::LocalLogProcess::Gui,
        ) {
            eprintln!("{error}");
        }
        configure_client_connection(app.handle(), data_dir)?;
        let settings = tauri::async_runtime::block_on(
            app.state::<usecase::client_connection::ClientConnectionUsecase>()
                .desktop_settings(),
        )?;
        app.manage(infrastructure::telemetry::init_telemetry(
            settings.crash_reporting,
            settings.performance_telemetry,
        ));
        apply_desktop_settings(app.handle(), settings);
        let initial_preferences = app
            .state::<infrastructure::platform::window_lifecycle::WindowPreferencesState>()
            .read();
        let authority =
            Arc::new(usecase::application_startup::ApplicationStartupAuthority::ready());
        app.manage(authority);
        infrastructure::platform::menu::setup_menu(app)?;
        infrastructure::platform::tray::setup_tray(app, |app| {
            infrastructure::platform::tray::mark_quit_requested();
            app.exit(0);
        })?;
        let handle = app.handle().clone();
        app.manage(Arc::new(
            adaptor::controller::application_lifecycle::ApplicationQuitIngress::new(
                move |intent| {
                    infrastructure::platform::tray::mark_quit_requested();
                    match intent {
                        usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code } => {
                            handle.exit(code)
                        }
                        usecase::shutdown_coordinator::ApplicationQuitIntent::Restart {
                            ..
                        } => handle.request_restart(),
                    }
                },
            ),
        ));
        let mut config =
            app.config().app.windows.first().cloned().ok_or_else(|| {
                std::io::Error::other("application window configuration is missing")
            })?;
        config.create = true;
        let window = if cfg!(feature = "performance-wdio") {
            app.get_webview_window(&config.label)
        } else {
            None
        };
        let window = match window {
            Some(window) => window,
            None => tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?,
        };
        infrastructure::platform::native_drop::install(&window);
        infrastructure::platform::window_lifecycle::apply_startup_window_preferences(
            app.handle(),
            initial_preferences,
        );
        record_window_ready();
        Ok(())
    });
    adaptor::controller::command::register_all(builder)
        .build(application_context())
        .expect("error while building tauri application")
        .run(infrastructure::platform::window_lifecycle::handle_run_event);
}

fn record_window_ready() {
    other::telemetry::record_startup_from_origin(other::telemetry::Startup::FirstWindowReady);
    other::telemetry::record_startup_from_origin(other::telemetry::Startup::AppStartup);
}

#[cfg(test)]
#[path = "desktop_test.rs"]
mod desktop_tests;
