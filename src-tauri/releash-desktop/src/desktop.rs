use crate::{adaptor, infrastructure, usecase};
use std::sync::Arc;
use tauri::Manager;

pub(crate) fn application_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

pub fn apply_desktop_settings<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: releashd::desktop_api::DesktopSettingsDto,
) {
    use releashd::desktop_api::TelemetryPort;
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
    releashd::desktop_api::TelemetryGateway.set_crash_reporting_enabled(settings.crash_reporting);
    releashd::desktop_api::TelemetryGateway.set_performance_enabled(settings.performance_telemetry);
}

pub fn run() {
    releashd::desktop_api::set_startup_origin(std::time::Instant::now());
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    let builder = builder.setup(|app| {
        infrastructure::platform::desktop_restart::wait_for_predecessor()?;
        let data_dir = releashd::desktop_api::default_data_dir_for_profile(
            releashd::desktop_api::BuildProfile::current(),
        )
        .ok_or("OS data directory is unavailable")?;
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
        if let Err(error) = releashd::desktop_api::init_local_log(
            &data_dir,
            releashd::desktop_api::LocalLogProcess::Gui,
        ) {
            eprintln!("{error}");
        }
        let hidden = std::env::args().any(|arg| arg == "--hidden");
        app.manage(usecase::cli_install::CliInstallUsecase(Arc::new(
            adaptor::gateway::cli_install::MacCliInstall,
        )));
        let connection = Arc::new(adaptor::gateway::daemon_connection::DaemonConnection::new(
            std::env::current_exe()?.with_file_name("releashd"),
            data_dir,
        ));
        let connection_usecase =
            Arc::new(usecase::daemon_connection::DaemonConnectionUsecase::new(
                connection.clone(),
                connection.clone(),
            ));
        app.manage(connection_usecase.clone());
        app.manage(usecase::login_item::LoginItemUsecase::new(
            Arc::new(adaptor::gateway::login_item::MacLoginItem),
            Arc::new(adaptor::gateway::login_item::DaemonLoginPreference(
                connection.clone(),
            )),
        ));
        app.manage(usecase::desktop_update::DesktopUpdateUsecase::new(
            Arc::new(adaptor::gateway::desktop_update::TauriUpdateGateway::new(
                app.handle().clone(),
            )),
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
        infrastructure::platform::tray::setup_tray(
            app,
            |app| adaptor::controller::desktop_lifecycle::quit(&app, 0),
            |app| {
                if let Err(error) = adaptor::controller::desktop_lifecycle::show(&app) {
                    log::error!("{error}");
                }
            },
            adaptor::controller::desktop_lifecycle::confirm_stop,
        )?;
        let handle = app.handle().clone();
        infrastructure::platform::native_termination::install(move || {
            adaptor::controller::desktop_lifecycle::quit(&handle, 0)
        })?;
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            adaptor::controller::desktop_lifecycle::initialize(&handle, hidden).await;
        });
        Ok(())
    });
    adaptor::controller::command::register_all(builder)
        .build(application_context())
        .expect("error while building tauri application")
        .run(adaptor::controller::desktop_lifecycle::handle_run_event);
}

pub fn record_window_ready() {
    releashd::desktop_api::record_startup_from_origin(
        releashd::desktop_api::Startup::FirstWindowReady,
    );
    releashd::desktop_api::record_startup_from_origin(releashd::desktop_api::Startup::AppStartup);
}

#[cfg(test)]
#[path = "desktop_test.rs"]
mod desktop_tests;
