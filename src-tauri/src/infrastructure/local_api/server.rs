use std::io;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::{http::StatusCode, routing::get};
use tokio::sync::oneshot;

use super::{process_start_time, LocalApiDiscovery, LocalApiDiscoveryFile, LocalApiServerError};

pub(crate) struct LocalApiServerBinding {
    listener: std::net::TcpListener,
    port: u16,
    token: Arc<str>,
    instance_id: String,
    discovery: LocalApiDiscoveryFile,
}

impl LocalApiServerBinding {
    pub(crate) fn bind(data_dir: PathBuf) -> Result<Self, LocalApiServerError> {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(LocalApiServerError::ListenerBind)?;
        let address = listener
            .local_addr()
            .map_err(LocalApiServerError::AddressResolution)?;
        if !address.ip().is_loopback() {
            return Err(LocalApiServerError::NonLoopback {
                address,
                source: io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    format!("bound address {address} is not loopback"),
                ),
            });
        }
        listener
            .set_nonblocking(true)
            .map_err(LocalApiServerError::Nonblocking)?;

        let token = Arc::<str>::from(generate_token());
        let instance_id = uuid::Uuid::new_v4().simple().to_string();
        let pid = std::process::id();
        let process_started_at = process_start_time(pid).ok_or_else(|| {
            LocalApiServerError::Discovery(io::Error::other(
                "failed to resolve local API process identity",
            ))
        })?;
        let discovery = LocalApiDiscoveryFile::create(
            &data_dir,
            LocalApiDiscovery {
                port: address.port(),
                token: token.to_string(),
                instance_id: instance_id.clone(),
                pid,
                process_started_at,
            },
        )
        .map_err(LocalApiServerError::Discovery)?;

        Ok(Self {
            listener,
            port: address.port(),
            token,
            instance_id,
            discovery,
        })
    }

    pub(crate) fn bearer_token(&self) -> Arc<str> {
        self.token.clone()
    }

    pub(crate) fn start(
        self,
        router: Router,
        runtime: &tokio::runtime::Handle,
    ) -> Arc<LocalApiServer> {
        let Self {
            listener,
            port,
            instance_id,
            discovery,
            ..
        } = self;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let discovery_for_task = discovery.clone();
        runtime.spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    log::error!("failed to initialize local API listener: {error}");
                    if let Err(error) = discovery_for_task.remove_if_owned() {
                        log::warn!("failed to remove local API discovery file: {error}");
                    }
                    return;
                }
            };
            let identity_path = format!("/.well-known/releash-local-api/{instance_id}");
            let router = Router::new()
                .route(&identity_path, get(|| async { StatusCode::NO_CONTENT }))
                .merge(router);
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
            if let Err(error) = result {
                log::error!("local API server stopped with an error: {error}");
            }
            if let Err(error) = discovery_for_task.remove_if_owned() {
                log::warn!("failed to remove local API discovery file: {error}");
            }
        });

        log::info!("local API listening on 127.0.0.1:{port}");
        Arc::new(LocalApiServer {
            shutdown: parking_lot::Mutex::new(Some(shutdown_tx)),
            discovery,
        })
    }
}

pub(crate) struct LocalApiServer {
    shutdown: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
    discovery: LocalApiDiscoveryFile,
}

impl LocalApiServer {
    pub(crate) fn shutdown(&self) {
        if let Some(sender) = self.shutdown.lock().take() {
            let _ = sender.send(());
        }
        if let Err(error) = self.discovery.remove_if_owned() {
            log::warn!("failed to remove local API discovery file: {error}");
        }
    }
}

impl Drop for LocalApiServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[cfg(test)]
#[path = "server_test.rs"]
mod server_tests;
