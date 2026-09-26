use crate::adaptor::protocol::{
    client as wire,
    connect::{rpc, to_rpc, to_wire},
};
use crate::usecase::client_connection::ClientConnectionDto;
use connectrpc::client::{ClientConfig, HttpClient};
use std::sync::Arc;

pub(crate) fn client(
    endpoint: &ClientConnectionDto,
) -> Result<rpc::ClientServiceClient<HttpClient>, String> {
    let config = ClientConfig::new(
        endpoint
            .url
            .parse()
            .map_err(|error| format!("Invalid client endpoint: {error}"))?,
    )
    .with_default_timeout(std::time::Duration::from_secs(30))
    .with_default_header("authorization", format!("Bearer {}", endpoint.token))
    .with_default_header("origin", "tauri://localhost");
    Ok(rpc::ClientServiceClient::new(
        HttpClient::plaintext(),
        config,
    ))
}

pub(crate) struct DesktopClient {
    client: Arc<rpc::ClientServiceClient<HttpClient>>,
    task: tokio::task::JoinHandle<()>,
    failure: Arc<parking_lot::Mutex<Option<String>>>,
}

impl Drop for DesktopClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl DesktopClient {
    pub fn start(client: rpc::ClientServiceClient<HttpClient>) -> Self {
        let client = Arc::new(client);
        let failure = Arc::new(parking_lot::Mutex::new(None));
        let monitor = client.clone();
        let failed = failure.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if let Err(error) = monitor
                    .get_server_info_with_options(
                        rpc::Unit::default(),
                        connectrpc::client::CallOptions::default()
                            .with_timeout(std::time::Duration::from_secs(5)),
                    )
                    .await
                {
                    if error.code == connectrpc::ErrorCode::ResourceExhausted {
                        continue;
                    }
                    *failed.lock() = Some(error_message(error));
                    break;
                }
            }
        });
        Self {
            client,
            task,
            failure,
        }
    }
    pub fn connected(&self) -> bool {
        !self.task.is_finished()
    }
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().clone()
    }
    pub async fn request(
        &self,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, String> {
        call(&self.client, command).await.map_err(error_message)
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
