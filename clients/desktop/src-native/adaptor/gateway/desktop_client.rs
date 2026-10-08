use crate::common::retry::{RetryBackoff, RetryLimiter};
use crate::domain::daemon_connection::{DaemonEndpoint, DaemonSubscription};
use connectrpc::client::{ClientConfig, HttpClient};
use futures_util::future::BoxFuture;
use releashd::desktop_api::descriptor;
use releashd::desktop_api::{call, rpc, to_wire, wire};
use releashd::desktop_api::{TechnicalFailure, TechnicalFailureNature};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

pub struct ConnectionPolicy {
    pub backoff: RetryBackoff,
    pub jitter: f64,
    reset_after: Duration,
    reconnect_codes: Vec<i32>,
    default_timeout: Duration,
}

pub static POLICY: LazyLock<ConnectionPolicy> = LazyLock::new(|| {
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
        default_timeout: Duration::from_millis(
            option("default_timeout_ms")
                .as_u32()
                .expect("default_timeout_ms")
                .into(),
        ),
    }
});

fn config(endpoint: &DaemonEndpoint) -> Result<ClientConfig, String> {
    Ok(ClientConfig::new(
        endpoint
            .url
            .parse()
            .map_err(|error| format!("Invalid client endpoint: {error}"))?,
    )
    .with_default_header("authorization", format!("Bearer {}", endpoint.token)))
}

pub fn client(endpoint: &DaemonEndpoint) -> Result<rpc::ClientServiceClient<HttpClient>, String> {
    Ok(rpc::ClientServiceClient::new(
        HttpClient::plaintext(),
        config(endpoint)?.with_default_timeout(POLICY.default_timeout),
    ))
}

pub fn stream_client(
    endpoint: &DaemonEndpoint,
) -> Result<rpc::ClientServiceClient<HttpClient>, String> {
    Ok(rpc::ClientServiceClient::new(
        HttpClient::plaintext(),
        config(endpoint)?,
    ))
}

type DesktopSettingsDto = releashd::desktop_api::DesktopSettingsDto;

pub struct DesktopClient {
    subscription: DaemonSubscription,
    client: Arc<rpc::ClientServiceClient<HttpClient>>,
    task: tokio::task::JoinHandle<()>,
    settings: tokio::sync::watch::Receiver<Option<DesktopSettingsDto>>,
    initial_settings: std::sync::OnceLock<DesktopSettingsDto>,
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
        subscription: DaemonSubscription,
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
            subscription,
            client: Arc::new(client),
            task,
            settings,
            rejected,
            initial_settings: std::sync::OnceLock::new(),
            exit,
        }
    }
    /// 購読で最初に届いた desktop 設定を待つ。届く前に購読の開始が再接続の対象でない失敗で終わったら、その失敗を返す。
    pub async fn first_settings(&self) -> Result<DesktopSettingsDto, TechnicalFailure> {
        let mut settings = self.settings.clone();
        let mut rejected = self.rejected.clone();
        let result = tokio::select! {
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
        };
        result.map(|value| *self.initial_settings.get_or_init(|| value))
    }
    pub fn subscription(&self) -> DaemonSubscription {
        self.subscription
    }
    pub fn initial_settings(&self) -> Option<DesktopSettingsDto> {
        self.initial_settings.get().copied()
    }
    pub fn current_settings(&self) -> Option<DesktopSettingsDto> {
        *self.settings.borrow()
    }
    pub fn settings_receiver(&self) -> tokio::sync::watch::Receiver<Option<DesktopSettingsDto>> {
        self.settings.clone()
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
    let mut reconnects = 0u64;
    loop {
        let attempt_started = tokio::time::Instant::now();
        let observed = observe(client, stream_client, settings, rejected).await;
        log::debug!("Desktop settings reconnect: {}", observed.message);
        if attempt_started.elapsed() > POLICY.reset_after {
            reconnects = 0;
        }
        reconnects = reconnects.saturating_add(1);
        if let Err(stopped) = limiter
            .wait_with_spread(POLICY.backoff, reconnects, POLICY.jitter)
            .await
        {
            return stopped.into();
        }
    }
}

fn subscription_result(
    result: Result<(), connectrpc::ConnectError>,
    rejected: &tokio::sync::watch::Sender<Option<TechnicalFailure>>,
) -> Option<TechnicalFailure> {
    let Err(error) = result else { return None };
    let reconnect = POLICY
        .reconnect_codes
        .contains(&(error.code.grpc_code() as i32));
    let observed = connection_failure(error);
    if reconnect {
        Some(observed)
    } else {
        log::warn!("Desktop settings subscription failed: {}", observed.message);
        rejected.send_replace(Some(observed));
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
pub struct SettingsSubscription<'a> {
    requested: bool,
    pub pending: Option<BoxFuture<'a, Result<(), connectrpc::ConnectError>>>,
}

impl<'a> SettingsSubscription<'a> {
    pub fn request_if_needed(
        &mut self,
        client: &'a rpc::ClientServiceClient<HttpClient>,
        client_id: &str,
    ) {
        if !self.requested {
            self.requested = true;
            self.pending = Some(start_settings_subscription(client, client_id.to_owned()));
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    async fn finish_pending(
        &mut self,
        rejected: &tokio::sync::watch::Sender<Option<TechnicalFailure>>,
    ) -> Option<TechnicalFailure> {
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
) -> TechnicalFailure {
    let client_id = uuid::Uuid::new_v4().to_string();
    let mut stream = match stream_client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: client_id.clone(),
            ..Default::default()
        })
        .await
    {
        Ok(stream) => stream,
        Err(error) => return connection_failure(error),
    };
    let mut subscription = SettingsSubscription::default();
    loop {
        let message = tokio::select! {
            result = subscription.finish_pending(rejected), if subscription.has_pending() => {
                if let Some(observed) = result {
                    return observed;
                }
                continue;
            }
            message = stream.message() => message,
        };
        let message = match message {
            Ok(Some(message)) => message,
            Ok(None) => {
                return TechnicalFailure {
                    nature: TechnicalFailureNature::Transient,
                    message: "State stream ended".into(),
                };
            }
            Err(error) => return connection_failure(error),
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

pub fn connection_failure(error: connectrpc::ConnectError) -> TechnicalFailure {
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

pub fn error_message(error: connectrpc::ConnectError) -> String {
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
            }
        })
        .unwrap_or_else(|| error.to_string())
}

#[cfg(test)]
#[path = "desktop_client_test.rs"]
mod desktop_client_tests;
