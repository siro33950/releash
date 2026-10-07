use crate::infrastructure::platform::window_lifecycle::{
    NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL,
};
use crate::usecase::daemon_connection::DaemonConnectionUsecase;
use std::sync::Arc;
use tauri::Manager;

pub(crate) fn show<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    let connection = app.state::<Arc<DaemonConnectionUsecase>>();
    let ready = connection.settings().is_some();
    let label = if ready {
        NORMAL_WINDOW_LABEL
    } else {
        STARTUP_FAILURE_WINDOW_LABEL
    };
    let window = match app.get_webview_window(label) {
        Some(window) => window,
        None => {
            let mut config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("Window configuration is missing")?;
            config.label = label.into();
            config.create = true;
            let window = tauri::WebviewWindowBuilder::from_config(app, &config)
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?;
            if ready {
                crate::infrastructure::platform::native_drop::install(&window);
            }
            if ready {
                crate::desktop::record_window_ready();
            }
            window
        }
    };
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

struct SettingsObserverStarted;

pub(crate) async fn initialize<R: tauri::Runtime>(app: &tauri::AppHandle<R>, hidden: bool) {
    match app.state::<Arc<DaemonConnectionUsecase>>().connect().await {
        Ok(()) => connected(app, hidden),
        Err(error) => {
            log::error!("{error}");
            let main = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Err(error) = show(&main) {
                    log::error!("{error}");
                }
            });
        }
    }
}

pub(crate) fn connected<R: tauri::Runtime>(app: &tauri::AppHandle<R>, hidden: bool) {
    let connection = app.state::<Arc<DaemonConnectionUsecase>>().inner().clone();
    let Some(settings) = connection.settings() else {
        return;
    };
    if app
        .try_state::<Option<releashd::desktop_api::TelemetryGuard>>()
        .is_none()
    {
        app.manage(releashd::desktop_api::init_telemetry(
            settings.crash_reporting,
            settings.performance_telemetry,
        ));
    }
    crate::desktop::apply_desktop_settings(app, settings);
    if let Err(error) = app
        .state::<crate::usecase::login_item::LoginItemUsecase>()
        .restore(settings.auto_launch)
    {
        log::error!("{error}");
    }
    let main = app.clone();
    let _ = app.run_on_main_thread(move || {
        let failed = main.get_webview_window(STARTUP_FAILURE_WINDOW_LABEL);
        if show_after_connection(hidden, settings.start_minimized, failed.is_some()) {
            if let Err(error) = show(&main) {
                log::error!("{error}");
            }
        }
        if let Some(failed) = failed {
            let _ = failed.destroy();
        }
    });
    if !app.manage(SettingsObserverStarted) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if let Some(settings) = connection.settings_update() {
                crate::desktop::apply_desktop_settings(&app, settings);
                if let Err(error) = app
                    .state::<crate::usecase::login_item::LoginItemUsecase>()
                    .restore(settings.auto_launch)
                {
                    log::error!("{error}");
                }
            }
        }
    });
}

type ConfirmationReply = Box<dyn FnOnce(bool) + Send>;

fn confirm_stop_with(
    confirm: impl FnOnce(&str, ConfirmationReply),
    stop: impl FnOnce() + Send + 'static,
) {
    confirm(
        "サーバを停止すると、動いている agent の Session も停止します。停止しますか？",
        Box::new(move |confirmed| {
            if confirmed {
                stop();
            }
        }),
    );
}

pub(crate) fn confirm_stop(app: tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let handle = app.clone();
    confirm_stop_with(
        |message, reply| {
            app.dialog()
                .message(message)
                .title("サーバを停止")
                .buttons(MessageDialogButtons::OkCancel)
                .show(reply)
        },
        move || {
            tauri::async_runtime::spawn(async move {
                if let Err(error) = handle.state::<Arc<DaemonConnectionUsecase>>().stop().await {
                    handle
                        .dialog()
                        .message(error.to_string())
                        .title("サーバを停止できませんでした")
                        .show(|_| {});
                }
            });
        },
    );
}

pub(crate) fn quit<R: tauri::Runtime>(app: &tauri::AppHandle<R>, code: i32) {
    crate::infrastructure::platform::native_termination::exit(app, code);
}

pub(crate) fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    crate::infrastructure::platform::window_lifecycle::handle_run_event(
        app,
        event,
        |code| quit(app, code),
        || {
            if let Err(error) = show(app) {
                log::error!("{error}");
            }
        },
    );
}

fn show_after_connection(hidden: bool, start_minimized: bool, failure_window: bool) -> bool {
    !hidden || !start_minimized || failure_window
}

#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
