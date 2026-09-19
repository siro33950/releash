use crate::{adaptor, infrastructure, other, usecase};
use std::sync::Arc;
use tauri::Manager;

pub(crate) fn application_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

pub(crate) fn apply_desktop_settings<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: usecase::app_config::query_service::DesktopSettingsDto,
) {
    use usecase::telemetry::TelemetryPort;
    let preferences = infrastructure::platform::window_lifecycle::WindowPreferences {
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
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(feature = "performance-wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());
    let builder = builder.setup(|app| {
        infrastructure::platform::desktop_restart::wait_for_predecessor()?;
        let data_dir = infrastructure::platform::app_data_dir::resolve_data_dir()?;
        let handle = app.handle().clone();
        let Some(lock) =
            infrastructure::platform::single_instance::acquire(&data_dir, move || {
                let handle = handle.clone();
                let main = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    if let Err(error) = adaptor::controller::desktop_lifecycle::show(&main) {
                        log::error!("{error}");
                    }
                });
            })?
        else {
            std::process::exit(0);
        };
        app.manage(lock);
        if let Err(error) = infrastructure::local_log::init(
            &data_dir,
            infrastructure::local_log::LocalLogProcess::Gui,
        ) {
            eprintln!("{error}");
        }
        let startup_config = adaptor::gateway::app_config::read_config_if_exists(&data_dir.join("releash.toml"));
        if let Err(reason) = &startup_config { log::error!("Startup preferences are unavailable; daemon initialization will report the failure: {reason}"); }
        let hidden = std::env::args().any(|arg| arg == "--hidden") && startup_config.as_ref().is_ok_and(|config| config.as_ref().is_some_and(|config| config.app.start_minimized));
        app.manage(usecase::cli_install::CliInstallUsecase(Arc::new(adaptor::gateway::cli_install::MacCliInstall)));
        let executable = std::env::current_exe()?.with_file_name("releash-backend");
        let files = Arc::new(adaptor::gateway::client_handoff::ClientHandoffFiles::new(data_dir.join("desktop-client-operations")));
        let handoff = Arc::new(usecase::client_handoff::ClientHandoffUsecase::new(files.clone(), files));
        app.manage(handoff.clone());
        let gateway = Arc::new(adaptor::gateway::daemon_supervision::DaemonProcessGateway::new(executable, data_dir, handoff));
        let login = usecase::login_item::LoginItemUsecase::new(Arc::new(adaptor::gateway::login_item::MacLoginItem), Arc::new(adaptor::gateway::login_item::DaemonLoginPreference(gateway.clone())));
        if let Ok(Some(config)) = &startup_config { if let Err(error) = login.restore(config.app.auto_launch) { log::error!("{error}"); } }
        app.manage(login);
        let supervisor = tauri::async_runtime::block_on(async {
            usecase::daemon_supervision::DaemonSupervisionUsecase::start(gateway)
        });
        app.manage(supervisor.clone());
        app.manage(usecase::desktop_update::DesktopUpdateUsecase::new(
            Arc::new(adaptor::gateway::desktop_update::TauriUpdateGateway::new(
                app.handle().clone(),
            )),
            supervisor.clone(),
        ));
        app.manage(usecase::client_connection::ClientConnectionUsecase(
            Box::new(supervisor.clone()),
        ));
        app.manage(Arc::new(
            usecase::application_startup::ApplicationStartupAuthority::ready(),
        ));
        app.manage(
            infrastructure::platform::window_lifecycle::WindowPreferencesState(
                parking_lot::RwLock::new(
                    infrastructure::platform::window_lifecycle::WindowPreferences {
                        close_to_tray: true,
                    },
                ),
            ),
        );
        infrastructure::platform::menu::setup_menu(app)?;
        infrastructure::platform::tray::setup_tray(app, |app| {
            adaptor::controller::desktop_lifecycle::request_quit(
                &app.state::<Arc<usecase::daemon_supervision::DaemonSupervisionUsecase>>(),
            );
        }, |app| {
            if let Err(error) = adaptor::controller::desktop_lifecycle::show(&app) { log::error!("{error}"); }
        })?;
        let native_quit = supervisor.clone();
        infrastructure::platform::native_termination::install(move || {
            adaptor::controller::desktop_lifecycle::request_quit(&native_quit);
        })?;
        let quit = supervisor.clone();
        app.manage(Arc::new(
            adaptor::controller::application_lifecycle::ApplicationQuitIngress::new(
                move |intent| {
                    let intent = match intent {
                        usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code } => {
                            crate::domain::daemon_supervision::StopIntent::Quit(code)
                        }
                        usecase::shutdown_coordinator::ApplicationQuitIntent::Restart {
                            ..
                        } => crate::domain::daemon_supervision::StopIntent::Restart,
                    };
                    if let Err(error) = quit.stop(intent) {
                        log::error!("{error}");
                    }
                },
            ),
        ));
        adaptor::controller::desktop_lifecycle::observe(app.handle().clone(), supervisor, hidden);
        Ok(())
    });
    adaptor::controller::command::register_all(builder)
        .build(application_context())
        .expect("error while building tauri application")
        .run(adaptor::controller::desktop_lifecycle::handle_run_event);
}

pub(crate) fn record_window_ready() {
    other::telemetry::record_startup_from_origin(other::telemetry::Startup::FirstWindowReady);
    other::telemetry::record_startup_from_origin(other::telemetry::Startup::AppStartup);
}

#[cfg(test)]
#[path = "desktop_test.rs"]
mod desktop_tests;
