use crate::adaptor::presenter::{
    client as wire,
    connect_wire::{rpc, to_rpc, to_wire},
};
use crate::common::retry::{bounded, RetryBackoff, RetryLimiter};
use crate::domain::daemon_supervision::DaemonLiveness;
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::client_connection::ClientConnectionDto;
use crate::usecase::failure::{
    attempt_expired, Failure, FailureKey, FailureOutput, WorkFailure, ATTEMPT_LIMIT,
};
use connectrpc::client::{ClientConfig, HttpClient};
use std::sync::Arc;

fn config(endpoint: &ClientConnectionDto) -> Result<ClientConfig, String> {
    Ok(ClientConfig::new(
        endpoint
            .url
            .parse()
            .map_err(|error| format!("Invalid client endpoint: {error}"))?,
    )
    .with_default_header("authorization", format!("Bearer {}", endpoint.token))
    .with_default_header("origin", "tauri://localhost"))
}

pub(crate) fn client(
    endpoint: &ClientConnectionDto,
) -> Result<rpc::ClientServiceClient<HttpClient>, String> {
    Ok(rpc::ClientServiceClient::new(
        HttpClient::plaintext(),
        config(endpoint)?.with_default_timeout(std::time::Duration::from_secs(30)),
    ))
}

pub(crate) fn stream_client(
    endpoint: &ClientConnectionDto,
) -> Result<rpc::ClientServiceClient<HttpClient>, String> {
    Ok(rpc::ClientServiceClient::new(
        HttpClient::plaintext(),
        config(endpoint)?,
    ))
}

type DesktopSettingsDto = crate::usecase::app_config::query_service::DesktopSettingsDto;

pub(crate) struct DesktopClient {
    client: Arc<rpc::ClientServiceClient<HttpClient>>,
    task: tokio::task::JoinHandle<()>,
    settings_task: tokio::task::JoinHandle<()>,
    settings: parking_lot::Mutex<tokio::sync::watch::Receiver<Option<DesktopSettingsDto>>>,
    failure: Arc<parking_lot::Mutex<Option<WorkFailure>>>,
}

impl Drop for DesktopClient {
    fn drop(&mut self) {
        self.task.abort();
        self.settings_task.abort();
    }
}

const DESKTOP_SETTINGS_TARGET: &str = "desktop-settings";

async fn receive_desktop_settings(
    client: &rpc::ClientServiceClient<HttpClient>,
    sender: &tokio::sync::watch::Sender<Option<DesktopSettingsDto>>,
) -> Result<(), String> {
    let client_id = uuid::Uuid::new_v4().to_string();
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: client_id.clone(),
            ..Default::default()
        })
        .await
        .map_err(error_message)?;
    while let Some(message) = stream.message().await.map_err(error_message)? {
        let event: wire::StateSubscriptionEvent =
            to_wire(&message.to_owned_message()).map_err(|error| error.to_string())?;
        use wire::state_subscription_event::Event;
        let payload = match event.event {
            Some(Event::Ready(_)) => {
                client
                    .start_state_subscription(rpc::StartStateSubscriptionRequest {
                        client_id: client_id.clone(),
                        target: DESKTOP_SETTINGS_TARGET.into(),
                        ..Default::default()
                    })
                    .await
                    .map_err(error_message)?;
                continue;
            }
            Some(Event::Snapshot(payload)) => payload,
            Some(Event::Change(change)) => match change.payload {
                Some(payload) => payload,
                None => continue,
            },
            _ => continue,
        };
        if let Some(wire::state_payload::Value::DesktopSettings(settings)) = payload.value {
            sender.send_replace(Some(settings.try_into()?));
        }
    }
    Err("Desktop settings stream ended".into())
}

impl DesktopClient {
    pub fn start(
        client: rpc::ClientServiceClient<HttpClient>,
        stream_client: rpc::ClientServiceClient<HttpClient>,
        key: FailureKey,
        failures: Arc<dyn FailureOutput>,
        limiter: Arc<RetryLimiter>,
    ) -> Self {
        let failure = Arc::new(parking_lot::Mutex::new(None));
        let (settings_sender, settings) = tokio::sync::watch::channel(None);
        let settings_client = stream_client.clone();
        let settings_task = tokio::spawn(async move {
            loop {
                if let Err(error) =
                    receive_desktop_settings(&settings_client, &settings_sender).await
                {
                    log::warn!("Desktop settings subscription interrupted: {error}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
        let failed = failure.clone();
        let task = tokio::spawn(async move {
            let lost = watch(&stream_client, &key, failures.as_ref(), &limiter).await;
            *failed.lock() = Some(lost);
        });
        Self {
            client: Arc::new(client),
            task,
            settings_task,
            settings: parking_lot::Mutex::new(settings),
            failure,
        }
    }
    pub fn connected(&self) -> bool {
        !self.task.is_finished()
    }
    /// 購読で最初に届いた desktop 設定を待つ。
    pub async fn first_settings(&self) -> Result<DesktopSettingsDto, String> {
        let mut receiver = self.settings.lock().clone();
        let settings = receiver
            .wait_for(|settings| settings.is_some())
            .await
            .map_err(|_| "Desktop settings are unavailable.".to_string())?
            .expect("waited for settings");
        Ok(settings)
    }
    pub fn current_settings(&self) -> Option<DesktopSettingsDto> {
        *self.settings.lock().borrow()
    }
    /// 前回の観測以降に届いた desktop 設定の変更を取り出す。
    pub fn settings_update(&self) -> Option<DesktopSettingsDto> {
        let mut receiver = self.settings.lock();
        if receiver.has_changed().unwrap_or(false) {
            *receiver.borrow_and_update()
        } else {
            None
        }
    }
    pub fn failure(&self) -> Option<WorkFailure> {
        self.failure.lock().clone()
    }
    pub async fn request(
        &self,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, String> {
        call(&self.client, command).await.map_err(error_message)
    }
}

async fn watch(
    client: &rpc::ClientServiceClient<HttpClient>,
    key: &FailureKey,
    failures: &dyn FailureOutput,
    limiter: &RetryLimiter,
) -> WorkFailure {
    let mut liveness = DaemonLiveness::default();
    loop {
        let failure = observe(client, &mut liveness, key, failures).await;
        failures.observed(key, failure.clone());
        if liveness.failed() {
            return failure;
        }
        limiter
            .wait(RetryBackoff::RECOVERY, liveness.consecutive_failures())
            .await;
    }
}

async fn observe(
    client: &rpc::ClientServiceClient<HttpClient>,
    liveness: &mut DaemonLiveness,
    key: &FailureKey,
    failures: &dyn FailureOutput,
) -> WorkFailure {
    let opened = bounded(ATTEMPT_LIMIT, attempt_expired, async {
        client
            .open_state_stream(rpc::OpenStateStreamRequest {
                client_id: uuid::Uuid::new_v4().to_string(),
                ..Default::default()
            })
            .await
            .map_err(liveness_failure)
    })
    .await;
    let mut stream = match opened {
        Ok(stream) => stream,
        Err(failure) => return failure,
    };
    loop {
        match tokio::time::timeout(
            ATTEMPT_LIMIT,
            stream.message::<rpc::StateSubscriptionEvent>(),
        )
        .await
        {
            Ok(Ok(Some(_))) => {
                if liveness.succeeded() {
                    failures.resolved(key);
                }
            }
            Ok(Ok(None)) => {
                return WorkFailure {
                    kind: Failure::Technical(TechnicalFailureNature::Transient),
                    message: "State stream ended".into(),
                }
            }
            Ok(Err(error)) => return liveness_failure(error),
            Err(_) => return attempt_expired(),
        }
    }
}

fn liveness_failure(error: connectrpc::ConnectError) -> WorkFailure {
    use connectrpc::ErrorCode;
    let nature = match error.code {
        ErrorCode::DeadlineExceeded => TechnicalFailureNature::TimedOut,
        ErrorCode::Unavailable | ErrorCode::ResourceExhausted => TechnicalFailureNature::Transient,
        ErrorCode::Canceled => TechnicalFailureNature::Cancelled,
        _ => TechnicalFailureNature::Other,
    };
    WorkFailure {
        kind: Failure::Technical(nature),
        message: error_message(error),
    }
}

pub(crate) async fn server_info(
    endpoint: &ClientConnectionDto,
) -> Result<wire::ServerInfo, String> {
    let response = client(endpoint)?
        .get_server_info(rpc::Unit::default())
        .await
        .map_err(error_message)?;
    to_wire(&response.into_owned()).map_err(|error| error.to_string())
}

fn error_message(error: connectrpc::ConnectError) -> String {
    use base64::Engine;
    use prost::Message;
    let decoder = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    error
        .details
        .iter()
        .filter(|detail| {
            detail.type_url.rsplit('/').next() == Some("releash.client.v1.CommandError")
        })
        .find_map(|detail| {
            let bytes = decoder.decode(detail.value.as_ref()?).ok()?;
            let detail = wire::CommandError::decode(bytes.as_slice()).ok()?;
            match detail.variant? {
                wire::command_error::Variant::Message(value) => value.value,
                wire::command_error::Variant::Coded(value) => value.message,
                wire::command_error::Variant::Application(value) => value.message,
            }
        })
        .unwrap_or_else(|| error.to_string())
}

#[cfg(test)]
#[path = "desktop_client_test.rs"]
mod desktop_client_tests;

include!(concat!(env!("OUT_DIR"), "/client_calls.rs"));
