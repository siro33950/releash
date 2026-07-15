use std::io;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use tokio::sync::oneshot;

use super::{LocalApiDiscovery, LocalApiDiscoveryFile, LocalApiServerError};

pub(crate) struct LocalApiServerBinding {
    listener: std::net::TcpListener,
    port: u16,
    token: Arc<str>,
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
        let discovery = LocalApiDiscoveryFile::create(
            &data_dir,
            LocalApiDiscovery {
                port: address.port(),
                token: token.to_string(),
                pid: std::process::id(),
            },
        )
        .map_err(LocalApiServerError::Discovery)?;

        Ok(Self {
            listener,
            port: address.port(),
            token,
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
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn generated_tokens_are_nonempty_and_change() {
        let first = generate_token();
        let second = generate_token();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
    }

    #[test]
    fn server_error_variants_preserve_their_sources() {
        let address: std::net::SocketAddr = "192.0.2.1:43123".parse().unwrap();
        let errors = [
            LocalApiServerError::ListenerBind(io::Error::other("bind")),
            LocalApiServerError::AddressResolution(io::Error::other("address")),
            LocalApiServerError::NonLoopback {
                address,
                source: io::Error::other("non-loopback"),
            },
            LocalApiServerError::Nonblocking(io::Error::other("nonblocking")),
            LocalApiServerError::Discovery(io::Error::other("discovery")),
        ];

        for error in errors {
            assert!(error.source().is_some(), "missing source for {error}");
        }
    }

    #[test]
    fn discovery_creation_failure_has_a_distinct_server_error() {
        let directory = tempfile::tempdir().unwrap();
        let data_path = directory.path().join("not-a-directory");
        std::fs::write(&data_path, "occupied").unwrap();

        let error = match LocalApiServerBinding::bind(data_path) {
            Ok(_) => panic!("discovery creation unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(matches!(error, LocalApiServerError::Discovery(_)));
        assert!(error.source().is_some());
    }

    #[tokio::test]
    async fn server_shutdown_signals_and_removes_owned_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let binding = LocalApiServerBinding::bind(directory.path().to_path_buf()).unwrap();
        let discovery_path = binding.discovery.path().to_path_buf();
        let server = binding.start(Router::new(), &tokio::runtime::Handle::current());

        assert!(discovery_path.exists());
        server.shutdown();
        tokio::task::yield_now().await;
        assert!(!discovery_path.exists());
    }
}
