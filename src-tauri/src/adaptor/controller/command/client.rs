use super::CommandRouter;

pub(crate) const COMMAND_NAMES: &[&str] = &[
    "complete_desktop_restoration",
    "fail_desktop_restoration",
    "apply_desktop_settings",
    "attach_desktop_client",
    "send_desktop_client_frame",
    "admit_client_command",
    "detach_desktop_client",
];

pub(crate) fn register<R: tauri::Runtime>(router: &mut CommandRouter<super::InvokeHandler<R>>) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(tauri::generate_handler![
            complete_desktop_restoration,
            fail_desktop_restoration,
            apply_desktop_settings,
            attach_desktop_client,
            send_desktop_client_frame,
            admit_client_command,
            detach_desktop_client
        ]),
    );
}

#[cfg(test)]
#[path = "client_test.rs"]
pub(crate) mod client_tests;

#[tauri::command]
fn apply_desktop_settings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    settings: crate::usecase::app_config::query_service::DesktopSettingsDto,
) {
    crate::desktop::apply_desktop_settings(&app, settings);
}

#[tauri::command]
async fn attach_desktop_client(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    channel: tauri::ipc::Channel<Option<Vec<u8>>>,
    attachment_id: String,
) -> Result<Vec<u8>, String> {
    let crate::usecase::daemon_supervision::DesktopAttachment {
        hello,
        mut frames,
        mut cancelled,
    } = supervisor
        .attach(attachment_id)
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(async move {
        loop {
            let frame =
                tokio::select! { frame = frames.recv() => frame, _ = &mut cancelled => break };
            match frame {
                Ok(frame) => {
                    let closed = frame.is_none();
                    if channel.send(frame).is_err() || closed {
                        break;
                    }
                }
                Err(_) => {
                    let _ = channel.send(None);
                    break;
                }
            }
        }
    });
    Ok(hello)
}

#[tauri::command]
async fn send_desktop_client_frame(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    bytes: Vec<u8>,
    launch_id: String,
) -> Result<(), crate::usecase::daemon_supervision::DesktopSendError> {
    use crate::adaptor::controller::api::protocol::client::{self as wire, envelope::Body};
    use crate::usecase::daemon_supervision::DesktopSendError as SendError;
    use prost::Message;
    let envelope = wire::Envelope::decode(bytes.as_slice()).map_err(SendError::not_sent)?;
    let command = match &envelope.body {
        Some(Body::Request(request)) => Some(request.as_ref()),
        Some(Body::OperationQuery(query)) => query.request.as_ref(),
        Some(Body::Heartbeat(_) | Body::Ack(_) | Body::RequestAck(_)) => None,
        _ => return Err(SendError::not_sent("Invalid desktop client frame")),
    };
    let name = command
        .map(|request| request.clone().into_value().map(|(name, _)| name))
        .transpose()
        .map_err(SendError::not_sent)?;
    let operation = if matches!(&envelope.body, Some(Body::OperationQuery(query)) if query.sent) {
        Some(crate::domain::daemon_supervision::ClientOperation::Recovery)
    } else {
        name.map(client_operation)
    };
    supervisor.send_frame(&launch_id, operation, bytes).await
}

#[tauri::command]
fn admit_client_command(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    command: String,
) -> Result<(), String> {
    supervisor
        .admit_client_command(client_operation(&command))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn detach_desktop_client(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    attachment_id: String,
) {
    supervisor.detach(&attachment_id);
}

fn client_operation(command: &str) -> crate::domain::daemon_supervision::ClientOperation {
    use crate::domain::daemon_supervision::ClientOperation;
    match command {
        "request_application_quit"
        | "get_application_quit_operation"
        | "get_application_shutdown"
        | "get_shutdown_plan"
        | "list_pending_application_attempts"
        | "resolve_shutdown_target_action"
        | "acknowledge_application_attempt" => ClientOperation::Shutdown,
        _ if matches!(
            crate::domain::client_operation::policy::recovery(command),
            crate::domain::client_operation::policy::Recovery::Read
                | crate::domain::client_operation::policy::Recovery::Connection
        ) =>
        {
            ClientOperation::RestoreState
        }
        _ => ClientOperation::Normal,
    }
}

#[tauri::command]
async fn complete_desktop_restoration(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    launch_id: String,
    attachment_id: String,
    generation: u64,
) -> Result<(), String> {
    supervisor
        .finish_restoration(&launch_id, &attachment_id, generation)
        .await
        .map_err(Into::into)
}

#[tauri::command]
fn fail_desktop_restoration(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    generation: u64,
    reason: String,
) {
    supervisor.fail_restoration(generation, reason);
}
