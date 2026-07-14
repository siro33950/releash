use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use tokio::sync::oneshot;

use super::{LocalApiDiscovery, LocalApiDiscoveryFile};

pub(crate) struct LocalApiServerBinding {
    listener: std::net::TcpListener,
    port: u16,
    token: Arc<str>,
    discovery: LocalApiDiscoveryFile,
}

impl LocalApiServerBinding {
    pub(crate) fn bind(data_dir: PathBuf) -> Result<Self, String> {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("failed to bind local API to 127.0.0.1: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("failed to resolve local API address: {error}"))?;
        if !address.ip().is_loopback() {
            return Err(format!(
                "local API unexpectedly bound to a non-loopback address: {address}"
            ));
        }
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("failed to make local API listener nonblocking: {error}"))?;

        let token = Arc::<str>::from(generate_token());
        let discovery = LocalApiDiscoveryFile::create(
            &data_dir,
            LocalApiDiscovery {
                port: address.port(),
                token: token.to_string(),
                pid: std::process::id(),
            },
        )
        .map_err(|error| format!("failed to write local API discovery file: {error}"))?;

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
    use super::*;

    #[test]
    fn generated_tokens_are_nonempty_and_change() {
        let first = generate_token();
        let second = generate_token();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
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
