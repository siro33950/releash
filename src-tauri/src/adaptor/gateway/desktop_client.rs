use crate::adaptor::presenter::{
    client as wire,
    connect_wire::{rpc, to_rpc, to_wire},
};
use crate::common::retry::{RetryBackoff, RetryLimiter};
use crate::domain::daemon_supervision::DaemonLiveness;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use crate::usecase::client_connection::ClientConnectionDto;
use connectrpc::client::{ClientConfig, HttpClient};
use futures_util::future::BoxFuture;
use prost_reflect::DescriptorPool;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

struct ConnectionPolicy {
    backoff: RetryBackoff,
    jitter: f64,
    reset_after: Duration,
    reconnect_codes: Vec<i32>,
    silence: Duration,
    default_timeout: Duration,
}

static POLICY: LazyLock<ConnectionPolicy> = LazyLock::new(|| {
    let pool = DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/client_descriptor.bin")).as_slice(),
    )
    .expect("client descriptors");
    let options = pool
        .get_service_by_name("releash.client.v1.ClientService")
        .expect("ClientService descriptor")
        .options();
    let option = |name: &str| {
        let extension = pool
            .get_extension_by_name(&format!("releash.client.v1.{name}"))
            .expect("client service option");
        options.get_extension(&extension).into_owned()
    };
    let backoff = option("connection_backoff");
    let backoff = backoff.as_message().expect("connection_backoff message");
    let millis = |name: &str| {
        Duration::from_millis(
            backoff
                .get_field_by_name(name)
                .and_then(|value| value.as_u32())
                .expect("connection_backoff field")
                .into(),
        )
    };
    let float = |name: &str| {
        backoff
            .get_field_by_name(name)
            .and_then(|value| value.as_f32())
            .expect("connection_backoff field") as f64
    };
    ConnectionPolicy {
        backoff: RetryBackoff::new(
            millis("initial_backoff_ms"),
            float("multiplier"),
            millis("max_backoff_ms"),
        ),
        jitter: float("jitter"),
        reset_after: millis("reset_after_ms"),
        reconnect_codes: option("reconnect_status_code")
            .as_list()
            .expect("reconnect_status_code list")
            .iter()
            .map(|code| code.as_enum_number().expect("reconnect status code"))
            .collect(),
        silence: Duration::from_millis(
            option("state_stream_silence_ms")
                .as_u32()
                .expect("state_stream_silence_ms")
                .into(),
        ),
        default_timeout: Duration::from_millis(
            option("default_timeout_ms")
                .as_u32()
                .expect("default_timeout_ms")
                .into(),
        ),
    }
});

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
        config(endpoint)?.with_default_timeout(POLICY.default_timeout),
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
    settings: parking_lot::Mutex<tokio::sync::watch::Receiver<Option<DesktopSettingsDto>>>,
    exit: Arc<parking_lot::Mutex<Option<WatchExit>>>,
}

enum WatchExit {
    Absent(TechnicalFailure),
    Unretryable(TechnicalFailure),
}

impl Drop for DesktopClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

const DESKTOP_SETTINGS_TARGET: &str = "desktop-settings";

impl DesktopClient {
    pub fn start(
        client: rpc::ClientServiceClient<HttpClient>,
        stream_client: rpc::ClientServiceClient<HttpClient>,
        limiter: Arc<RetryLimiter>,
    ) -> Self {
        let exit = Arc::new(parking_lot::Mutex::new(None));
        let (settings_sender, settings) = tokio::sync::watch::channel(None);
        let observed_exit = exit.clone();
        let call_client = client.clone();
        let task = tokio::spawn(async move {
            let result = watch(&call_client, &stream_client, &settings_sender, &limiter).await;
            if let WatchExit::Unretryable(failure) = &result {
                log::warn!("State stream cannot reconnect: {}", failure.message);
            }
            *observed_exit.lock() = Some(result);
        });
        Self {
            client: Arc::new(client),
            task,
            settings: parking_lot::Mutex::new(settings),
            exit,
        }
    }
    pub fn connected(&self) -> bool {
        match self.exit.lock().as_ref() {
            Some(WatchExit::Absent(_)) => false,
            Some(WatchExit::Unretryable(_)) => true,
            None => !self.task.is_finished(),
        }
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
    pub fn failure(&self) -> Option<TechnicalFailure> {
        match self.exit.lock().as_ref() {
            Some(WatchExit::Absent(failure)) => Some(failure.clone()),
            _ => None,
        }
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
    stream_client: &rpc::ClientServiceClient<HttpClient>,
    settings: &tokio::sync::watch::Sender<Option<DesktopSettingsDto>>,
    limiter: &RetryLimiter,
) -> WatchExit {
    let mut liveness = DaemonLiveness::default();
    let mut reconnects = 0u64;
    loop {
        let attempt_started = tokio::time::Instant::now();
        let observed = observe(client, stream_client, settings, &mut liveness).await;
        if observed.liveness && liveness.failed() {
            return WatchExit::Absent(observed.failure);
        }
        if !observed.reconnect {
            return WatchExit::Unretryable(observed.failure);
        }
        if attempt_started.elapsed() > POLICY.reset_after {
            reconnects = 0;
        }
        reconnects = reconnects.saturating_add(1);
        limiter
            .wait_with_spread(POLICY.backoff, reconnects, POLICY.jitter)
            .await;
    }
}

struct Observation {
    failure: TechnicalFailure,
    liveness: bool,
    reconnect: bool,
}

fn connection_error(error: connectrpc::ConnectError, liveness: bool) -> Observation {
    let reconnect = POLICY
        .reconnect_codes
        .contains(&(error.code.grpc_code() as i32));
    Observation {
        failure: liveness_failure(error),
        liveness,
        reconnect,
    }
}

async fn observe(
    client: &rpc::ClientServiceClient<HttpClient>,
    stream_client: &rpc::ClientServiceClient<HttpClient>,
    settings: &tokio::sync::watch::Sender<Option<DesktopSettingsDto>>,
    liveness: &mut DaemonLiveness,
) -> Observation {
    let client_id = uuid::Uuid::new_v4().to_string();
    let opened = tokio::time::timeout(
        POLICY.silence,
        stream_client.open_state_stream(rpc::OpenStateStreamRequest {
            client_id: client_id.clone(),
            ..Default::default()
        }),
    )
    .await;
    let mut stream = match opened {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => return connection_error(error, true),
        Err(_) => return silence_failure(),
    };
    let mut bookmark_deadline = tokio::time::Instant::now() + POLICY.silence;
    let mut subscription_start: Option<BoxFuture<'_, Result<(), connectrpc::ConnectError>>> = None;
    let mut subscription_requested = false;
    loop {
        let message = tokio::select! {
            result = async { subscription_start.as_mut().expect("pending subscription").await }, if subscription_start.is_some() => {
                subscription_start = None;
                if let Err(error) = result {
                    let observed = connection_error(error, false);
                    if observed.reconnect {
                        return observed;
                    }
                    log::warn!("Desktop settings subscription failed: {}", observed.failure.message);
                }
                continue;
            }
            message = tokio::time::timeout_at(bookmark_deadline, stream.message()) => message,
        };
        let message = match message {
            Ok(Ok(Some(message))) => message,
            Ok(Ok(None)) => {
                return Observation {
                    failure: TechnicalFailure {
                        nature: TechnicalFailureNature::Transient,
                        message: "State stream ended".into(),
                    },
                    liveness: true,
                    reconnect: true,
                };
            }
            Ok(Err(error)) => return connection_error(error, true),
            Err(_) => return silence_failure(),
        };
        let event: wire::StateSubscriptionEvent = match to_wire(&message.to_owned_message()) {
            Ok(event) => event,
            Err(error) => {
                log::warn!("State event decode failed: {error}");
                continue;
            }
        };
        use wire::state_subscription_event::Event;
        let payload = match event.event {
            Some(Event::Bookmark(_)) => {
                liveness.succeeded();
                bookmark_deadline = tokio::time::Instant::now() + POLICY.silence;
                continue;
            }
            Some(Event::Ready(_)) => {
                if !subscription_requested {
                    subscription_requested = true;
                    subscription_start = Some(Box::pin(async {
                        client
                            .start_state_subscription(rpc::StartStateSubscriptionRequest {
                                client_id: client_id.clone(),
                                target: DESKTOP_SETTINGS_TARGET.into(),
                                ..Default::default()
                            })
                            .await
                            .map(|_| ())
                    }));
                }
                continue;
            }
            Some(Event::Snapshot(payload)) => payload,
            Some(Event::Change(change)) => match change.payload {
                Some(payload) => payload,
                None => continue,
            },
            _ => continue,
        };
        if let Some(wire::state_payload::Value::DesktopSettings(value)) = payload.value {
            match value.try_into() {
                Ok(value) => {
                    settings.send_replace(Some(value));
                }
                Err(error) => log::warn!("Desktop settings decode failed: {error}"),
            }
        }
    }
}

fn silence_failure() -> Observation {
    Observation {
        failure: TechnicalFailure {
            nature: TechnicalFailureNature::TimedOut,
            message: "State stream was silent".into(),
        },
        liveness: true,
        reconnect: true,
    }
}

fn liveness_failure(error: connectrpc::ConnectError) -> TechnicalFailure {
    use connectrpc::ErrorCode;
    let nature = match error.code {
        ErrorCode::DeadlineExceeded => TechnicalFailureNature::TimedOut,
        ErrorCode::Unavailable | ErrorCode::ResourceExhausted | ErrorCode::Aborted => {
            TechnicalFailureNature::Transient
        }
        ErrorCode::Canceled => TechnicalFailureNature::Cancelled,
        _ => TechnicalFailureNature::Other,
    };
    TechnicalFailure {
        nature,
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
