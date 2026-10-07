use connectrpc::client::{ClientConfig, HttpClient};
use connectrpc::{ConnectError, ErrorCode};
use releash_sdk::compatibility::Compatibility;
use releash_sdk::discovery::{lookup_process_start_time, read};
use releash_sdk::{rpc, wire};
use std::path::Path;
use std::time::Duration;

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
    let discovery = read(data_dir)?;
    discovery.verify_process(lookup_process_start_time)?;
    let config = ClientConfig::new(
        format!("http://127.0.0.1:{}", discovery.port)
            .parse()
            .map_err(|error| {
                ConnectError::unavailable(format!("Invalid client endpoint: {error}"))
            })?,
    )
    .with_default_header(
        "authorization",
        format!(
            "Bearer {}",
            token.unwrap_or_else(|| discovery.token.clone())
        ),
    )
    .with_default_timeout(Duration::from_secs(5));
    let client = Client::new(HttpClient::plaintext(), config);
    let response = client.get_server_info(rpc::Unit::default()).await?;
    let info: wire::ServerInfo = to_wire(&response.into_owned())?;
    discovery.verify_server(&info)?;
    match Compatibility::assess(releash_sdk::descriptor::protocol(), info.protocol) {
        Compatibility::Compatible => Ok(client),
        compatibility => Err(ConnectError::new(
            ErrorCode::FailedPrecondition,
            format!(
                "{} (client release {}, server release {})",
                if compatibility == Compatibility::ServerOlder {
                    "server is older"
                } else {
                    "client is older"
                },
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
    tokio::time::timeout(Duration::from_secs(5), async {
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
