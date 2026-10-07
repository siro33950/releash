use crate::adaptor::presenter::desktop_lifecycle::TauriDesktopLifecycle;
use crate::common::log_failure::LogFailure;
use crate::usecase::desktop_lifecycle::DesktopLifecycleUsecase;
use std::sync::Arc;
use tauri::Manager;
pub(crate) fn show<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    app.state::<TauriDesktopLifecycle<R>>()
        .show(app.state::<Arc<DesktopLifecycleUsecase>>().show())
}
pub(crate) async fn initialize<R: tauri::Runtime>(app: &tauri::AppHandle<R>, hidden: bool) {
    let result = connection(app, |lifecycle, failure_window| {
        Box::pin(async move {
            lifecycle
                .initialize(Some(hidden), failure_window)
                .await
                .map(|connected| ((), connected))
        })
    })
    .await;
    if let Err(error) = result {
        app.state::<TauriDesktopLifecycle<R>>()
            .connection_failed(error);
    }
}

pub(crate) async fn connection<R: tauri::Runtime, T>(
    app: &tauri::AppHandle<R>,
    operation: impl for<'a> FnOnce(
        &'a DesktopLifecycleUsecase,
        bool,
    ) -> crate::domain::daemon_connection::DaemonResult<
        'a,
        (T, crate::usecase::desktop_lifecycle::ConnectedDesktop),
    >,
) -> Result<T, crate::domain::daemon_connection::DaemonConnectionFailure> {
    crate::common::serial::run(
        app,
        async {
            let presenter = app.state::<TauriDesktopLifecycle<R>>();
            let lifecycle = app.state::<Arc<DesktopLifecycleUsecase>>();
            operation(&lifecycle, presenter.failure_window()).await
        },
        |result| {
            result.map(|(value, output)| {
                connected(app, output);
                value
            })
        },
    )
    .await
}

pub(crate) fn connected<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    connected: crate::usecase::desktop_lifecycle::ConnectedDesktop,
) {
    app.state::<LogFailure<Arc<DesktopLifecycleUsecase>>>()
        .record(&connected.restoration);
    app.state::<TauriDesktopLifecycle<R>>().connected(connected);
}

pub(crate) async fn settings_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    client: Arc<crate::adaptor::gateway::desktop_client::DesktopClient>,
    settings: releashd::desktop_api::DesktopSettingsDto,
) {
    crate::common::serial::run(
        app,
        async {
            app.state::<Arc<crate::adaptor::gateway::daemon_connection::DaemonServiceGateway>>()
                .client()
                .is_ok_and(|current| Arc::ptr_eq(&current, &client))
        },
        |current| {
            if current {
                let _ = app
                    .state::<LogFailure<Arc<DesktopLifecycleUsecase>>>()
                    .call(|lifecycle| lifecycle.settings_changed(settings));
                app.state::<TauriDesktopLifecycle<R>>()
                    .apply_settings(settings);
            }
        },
    )
    .await;
}

pub(crate) fn confirm_stop(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let presenter = app.state::<TauriDesktopLifecycle<tauri::Wry>>();
        let lifecycle = app.state::<Arc<DesktopLifecycleUsecase>>();
        let result =
            stop_after_confirmation(presenter.confirm_stop().await, || lifecycle.stop()).await;
        if let Err(error) = result {
            presenter.stop_failed(error);
        }
    });
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

async fn stop_after_confirmation<
    F: std::future::Future<
        Output = Result<(), crate::domain::daemon_connection::DaemonConnectionFailure>,
    >,
>(
    confirmed: Result<bool, String>,
    stop: impl FnOnce() -> F,
) -> Result<(), crate::domain::daemon_connection::DaemonConnectionFailure> {
    if confirmed
        .map_err(crate::domain::daemon_connection::DaemonConnectionFailure::TechnicalFailure)?
    {
        stop().await?;
    }
    Ok(())
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
