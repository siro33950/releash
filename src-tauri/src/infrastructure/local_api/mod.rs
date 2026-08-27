mod client;
mod discovery;
mod server;

pub(crate) use client::{
    read_local_api_discovery, LocalApiDiscoveryReadError, LocalApiHttpClient,
    LocalApiIdentityRequestError, LocalApiTransportError,
};
pub(crate) use discovery::{
    local_api_discovery_path, lookup_process_start_time, process_start_time, LocalApiDiscovery,
    LocalApiDiscoveryFile, ProcessStartTimeLookup,
};
#[cfg(debug_assertions)]
pub(crate) use server::LocalApiServer;
pub(crate) use server::LocalApiServerBinding;

#[derive(Debug, thiserror::Error)]
pub(crate) enum LocalApiServerError {
    #[error("failed to bind local API to 127.0.0.1: {0}")]
    ListenerBind(#[source] std::io::Error),
    #[error("failed to resolve local API address: {0}")]
    AddressResolution(#[source] std::io::Error),
    #[error("local API unexpectedly bound to a non-loopback address: {address}")]
    NonLoopback {
        address: std::net::SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to make local API listener nonblocking: {0}")]
    Nonblocking(#[source] std::io::Error),
    #[error("failed to write local API discovery file: {0}")]
    Discovery(#[source] std::io::Error),
}
