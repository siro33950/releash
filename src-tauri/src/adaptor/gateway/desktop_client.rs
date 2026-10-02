use crate::adaptor::presenter::client::descriptor;
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
    let options = descriptor::pool()
        .get_service_by_name("releash.client.v1.ClientService")
        .expect("ClientService descriptor")
        .options();
    let option = |name: &str| descriptor::option(&options, name);
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
    rejected: tokio::sync::watch::Receiver<Option<TechnicalFailure>>,
    exit: Arc<parking_lot::Mutex<Option<TechnicalFailure>>>,
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
        let (rejected_sender, rejected) = tokio::sync::watch::channel(None);
        let observed_exit = exit.clone();
        let call_client = client.clone();
        let task = tokio::spawn(async move {
            let result = watch(
                &call_client,
                &stream_client,
                &settings_sender,
                &rejected_sender,
                &limiter,
            )
            .await;
            *observed_exit.lock() = Some(result);
        });
        Self {
            client: Arc::new(client),
            task,
            settings: parking_lot::Mutex::new(settings),
            rejected,
            exit,
        }
    }
    pub fn connected(&self) -> bool {
        self.exit.lock().is_none() && !self.task.is_finished()
    }
    /// 購読で最初に届いた desktop 設定を待つ。届く前に購読の開始が再接続の対象でない失敗で終わったら、その失敗を返す。
    pub async fn first_settings(&self) -> Result<DesktopSettingsDto, TechnicalFailure> {
        let mut settings = self.settings.lock().clone();
        let mut rejected = self.rejected.clone();
        tokio::select! {
            biased;
            result = settings.wait_for(|settings| settings.is_some()) => match result {
                Ok(settings) => Ok(settings.expect("waited for settings")),
                Err(_) => Err(self.failure().unwrap_or_else(|| TechnicalFailure {
                    nature: TechnicalFailureNature::Other,
                    message: "Desktop settings are unavailable.".into(),
                })),
            },
            Ok(failure) = rejected.wait_for(|failure| failure.is_some()) => {
                Err(failure.clone().expect("waited for failure"))
            }
        }
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
        self.exit.lock().clone()
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
    rejected: &tokio::sync::watch::Sender<Option<TechnicalFailure>>,
    limiter: &RetryLimiter,
) -> TechnicalFailure {
    let mut liveness = DaemonLiveness::default();
    let mut reconnects = 0u64;
    loop {
        let attempt_started = tokio::time::Instant::now();
        let observed = observe(client, stream_client, settings, rejected, &mut liveness).await;
        if observed.liveness && liveness.failed() {
            return observed.failure;
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
}

fn connection_error(error: connectrpc::ConnectError, liveness: bool) -> Observation {
    Observation {
        failure: liveness_failure(error),
        liveness,
    }
}

fn subscription_result(
    result: Result<(), connectrpc::ConnectError>,
    rejected: &tokio::sync::watch::Sender<Option<TechnicalFailure>>,
) -> Option<Observation> {
    let Err(error) = result else { return None };
    let reconnect = POLICY
        .reconnect_codes
        .contains(&(error.code.grpc_code() as i32));
    let observed = connection_error(error, false);
    if reconnect {
        Some(observed)
    } else {
        log::warn!(
            "Desktop settings subscription failed: {}",
            observed.failure.message
        );
        rejected.send_replace(Some(observed.failure));
        None
    }
}

fn start_settings_subscription(
    client: &rpc::ClientServiceClient<HttpClient>,
    client_id: String,
) -> BoxFuture<'_, Result<(), connectrpc::ConnectError>> {
    Box::pin(async move {
        client
            .start_state_subscription(rpc::StartStateSubscriptionRequest {
                subscription_id: uuid::Uuid::new_v4().to_string(),
                client_id,
                target: DESKTOP_SETTINGS_TARGET.into(),
                ..Default::default()
            })
            .await
            .map(|_| ())
    })
}

#[derive(Default)]
struct SettingsSubscription<'a> {
    requested: bool,
    pending: Option<BoxFuture<'a, Result<(), connectrpc::ConnectError>>>,
}

impl<'a> SettingsSubscription<'a> {
    fn request_if_needed(
        &mut self,
        client: &'a rpc::ClientServiceClient<HttpClient>,
        client_id: &str,
    ) {
        if !self.requested {
            self.requested = true;
            self.pending = Some(start_settings_subscription(client, client_id.to_owned()));
        }
    }

    fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    async fn finish_pending(
        &mut self,
        rejected: &tokio::sync::watch::Sender<Option<TechnicalFailure>>,
    ) -> Option<Observation> {
        let result = self.pending.as_mut().expect("pending subscription").await;
        self.pending = None;
        subscription_result(result, rejected)
    }
}

fn apply_settings(
    payload: wire::StatePayload,
    settings: &tokio::sync::watch::Sender<Option<DesktopSettingsDto>>,
) {
    if let Some(wire::state_payload::Value::DesktopSettings(value)) = payload.value {
        match value.try_into() {
            Ok(value) => {
                settings.send_replace(Some(value));
            }
            Err(error) => log::warn!("Desktop settings decode failed: {error}"),
        }
    }
}

async fn observe(
    client: &rpc::ClientServiceClient<HttpClient>,
    stream_client: &rpc::ClientServiceClient<HttpClient>,
    settings: &tokio::sync::watch::Sender<Option<DesktopSettingsDto>>,
    rejected: &tokio::sync::watch::Sender<Option<TechnicalFailure>>,
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
    let mut subscription = SettingsSubscription::default();
    loop {
        let message = tokio::select! {
            result = subscription.finish_pending(rejected), if subscription.has_pending() => {
                if let Some(observed) = result {
                    return observed;
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
                subscription.request_if_needed(client, &client_id);
                continue;
            }
            Some(Event::Snapshot(payload)) => payload,
            Some(Event::Change(change)) => match change.payload {
                Some(payload) => payload,
                None => continue,
            },
            _ => continue,
        };
        apply_settings(payload, settings);
    }
}

fn silence_failure() -> Observation {
    Observation {
        failure: TechnicalFailure {
            nature: TechnicalFailureNature::TimedOut,
            message: "State stream was silent".into(),
        },
        liveness: true,
    }
}

fn liveness_failure(error: connectrpc::ConnectError) -> TechnicalFailure {
    use connectrpc::ErrorCode;
    let nature = match error.code {
        ErrorCode::DeadlineExceeded => TechnicalFailureNature::TimedOut,
        ErrorCode::Unavailable | ErrorCode::ResourceExhausted => TechnicalFailureNature::Transient,
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
