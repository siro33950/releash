use crate::compatibility::Compatibility;
use crate::daemon;
use crate::{rpc, wire};
use connectrpc::client::HttpClient;
use connectrpc::{ConnectError, ErrorCode};
use std::path::Path;

pub type Client = rpc::ClientServiceClient<HttpClient>;

pub fn to_rpc<T: buffa::Message>(value: &impl prost::Message) -> Result<T, ConnectError> {
    T::decode_from_slice(&prost::Message::encode_to_vec(value))
        .map_err(|error| ConnectError::internal(error.to_string()))
}

pub fn to_wire<T: prost::Message + Default>(
    value: &impl buffa::Message,
) -> Result<T, ConnectError> {
    T::decode(buffa::Message::encode_to_vec(value).as_slice())
        .map_err(|error| ConnectError::internal(error.to_string()))
}

pub async fn connect(data_dir: &Path, token: Option<String>) -> Result<Client, ConnectError> {
    let discovery = match daemon::running(data_dir).map_err(daemon_error)? {
        Some(discovery) => discovery,
        None => return Err(ConnectError::unavailable("server is not running")),
    };
    let token = token.as_deref().unwrap_or(&discovery.token);
    let client = daemon::client(&discovery, token);
    let info = daemon::server_info(&discovery, token)
        .await
        .map_err(daemon_error)?;
    match Compatibility::assess(crate::descriptor::protocol(), info.protocol) {
        Compatibility::Compatible => Ok(client),
        compatibility => Err(ConnectError::new(
            ErrorCode::FailedPrecondition,
            format!(
                "{} (client release {}, server release {})",
                compatibility_guidance(compatibility),
                env!("CARGO_PKG_VERSION"),
                info.release
            ),
        )),
    }
}

pub async fn snapshot(
    client: &Client,
    target: &str,
    args: Vec<String>,
) -> Result<wire::StatePayload, ConnectError> {
    tokio::time::timeout(daemon::timeout("default_timeout_ms"), async {
        let client_id = uuid::Uuid::new_v4().to_string();
        let subscription_id = uuid::Uuid::new_v4().to_string();
        let mut stream = client
            .open_state_stream(rpc::OpenStateStreamRequest {
                client_id: client_id.clone(),
                ..Default::default()
            })
            .await?;
        loop {
            let message = stream
                .message()
                .await?
                .ok_or_else(|| ConnectError::unavailable("State stream ended"))?;
            let event: wire::StateSubscriptionEvent = to_wire(&message.to_owned_message())?;
            use wire::state_subscription_event::Event;
            match event.event {
                Some(Event::Ready(_)) => {
                    client
                        .start_state_subscription(rpc::StartStateSubscriptionRequest {
                            client_id: client_id.clone(),
                            subscription_id: subscription_id.clone(),
                            target: target.into(),
                            args: args.clone(),
                            ..Default::default()
                        })
                        .await?;
                }
                Some(Event::Snapshot(payload)) => return Ok(payload),
                Some(Event::Failure(failure)) => {
                    return Err(ConnectError::new(
                        ErrorCode::from_grpc_code(failure.code as u32)
                            .unwrap_or(ErrorCode::Unknown),
                        failure.message,
                    ))
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| ConnectError::unavailable("State subscription timed out"))?
}

pub fn compatibility_guidance(compatibility: Compatibility) -> &'static str {
    match compatibility {
        Compatibility::Compatible => "compatible",
        Compatibility::ServerOlder => {
            "server is older; run `releash server restart` to update the server"
        }
        Compatibility::ClientOlder => "client is older; update the Releash client",
    }
}

pub fn daemon_error(error: daemon::DaemonError) -> ConnectError {
    match error {
        daemon::DaemonError::Connect(error) => error,
        error => ConnectError::unavailable(error.to_string()),
    }
}

pub fn error_guidance(mut error: ConnectError) -> ConnectError {
    if error.code == ErrorCode::Unimplemented {
        let message = error
            .message
            .get_or_insert_with(|| "RPC is unimplemented".into());
        message.push_str("; run `releash server restart` to update the server");
    }
    error
}
