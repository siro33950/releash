#[cfg(feature = "desktop")]
use serde::Serialize;

pub const CLIENT_WS_PATH: &str = "/v1/client";

#[cfg(feature = "desktop")]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientEndpoint {
    pub url: String,
    pub auth_subprotocol: String,
}

#[cfg(all(test, feature = "desktop"))]
#[path = "client_test.rs"]
mod client_tests;
