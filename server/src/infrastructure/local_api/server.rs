use std::io;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tokio::sync::oneshot;

use super::{LocalApiDiscovery, LocalApiDiscoveryFile, LocalApiServerError};

const LOCAL_API_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct LocalApiServerBinding {
    listener: std::net::TcpListener,
    port: u16,
    terminal_token: super::BearerToken,
    hook_token: super::BearerToken,
    client_discovery: LocalApiDiscoveryFile,
}

impl LocalApiServerBinding {
    #[cfg(feature = "test-support")]
    pub fn test_discovery_path(&self) -> &std::path::Path {
        self.client_discovery.path()
    }

    pub(crate) fn bind(
        data_dir: PathBuf,
        daemon_id: String,
        pid: u32,
        process_started_at: u64,
        hook_token: super::BearerToken,
    ) -> Result<Self, LocalApiServerError> {
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

        let terminal_token = Arc::<str>::from(generate_token());
        let client_discovery = LocalApiDiscoveryFile::prepare_named(
            &data_dir,
            "client-api.json",
            LocalApiDiscovery {
                port: address.port().into(),
                token: terminal_token.to_string(),
                daemon_id: daemon_id.clone(),
                pid,
                process_started_at,
                ..Default::default()
            },
        );

        Ok(Self {
            listener,
            port: address.port(),
            terminal_token: terminal_token.into(),
            hook_token,
            client_discovery,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn terminal_bearer_token(&self) -> Arc<str> {
        self.terminal_token.token()
    }

    pub fn client_bearer_token(&self) -> super::BearerToken {
        self.terminal_token.clone()
    }

    #[cfg(feature = "test-support")]
    pub fn test_hook_token(&self) -> Arc<str> {
        self.hook_token.token()
    }

    pub fn hook_bearer_token(&self) -> super::BearerToken {
        self.hook_token.clone()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub fn start(
        self,
        router: Router,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Arc<LocalApiServer>, LocalApiServerError> {
        let Self {
            listener,
            port,
            client_discovery,
            terminal_token,
            hook_token,
            ..
        } = self;
        let _runtime = runtime.enter();
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(LocalApiServerError::ListenerBind)?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_discovery_for_task = client_discovery.clone();
        let task = runtime.spawn(async move {
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
            if let Err(error) = result {
                log::error!("local API server stopped with an error: {error}");
            }
            if let Err(error) = client_discovery_for_task.remove_if_owned() {
                log::warn!("failed to remove client discovery file: {error}");
            }
        });

        log::info!("local API listening on 127.0.0.1:{port}");
        Ok(Arc::new(LocalApiServer {
            shutdown: parking_lot::Mutex::new(Some(shutdown_tx)),
            task: parking_lot::Mutex::new(Some(task)),
            terminal_token,
            hook_token,
            client_discovery,
        }))
    }
}

pub struct LocalApiServer {
    terminal_token: super::BearerToken,
    hook_token: super::BearerToken,
    shutdown: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
    task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
    client_discovery: LocalApiDiscoveryFile,
}

impl LocalApiServer {
    pub fn publish_discovery(&self) -> Result<(), LocalApiServerError> {
        let result = self.client_discovery.publish();
        if let Err(error) = result {
            self.shutdown();
            return Err(LocalApiServerError::Discovery(error));
        }
        Ok(())
    }
    pub fn shutdown(&self) {
        self.terminal_token.revoke();
        self.hook_token.revoke();
        if let Some(sender) = self.shutdown.lock().take() {
            let _ = sender.send(());
        }
        if let Err(error) = self.client_discovery.remove_if_owned() {
            log::warn!("failed to remove client discovery file: {error}");
        }
    }

    pub async fn shutdown_and_wait(&self) -> Result<(), tokio::task::JoinError> {
        self.shutdown();
        let task = self.task.lock().take();
        if let Some(task) = task {
            wait_for_server_task(task, LOCAL_API_SHUTDOWN_TIMEOUT).await?;
        }
        Ok(())
    }
}

pub async fn wait_for_server_task(
    mut task: tokio::task::JoinHandle<()>,
    timeout: Duration,
) -> Result<(), tokio::task::JoinError> {
    if let Ok(result) = tokio::time::timeout(timeout, &mut task).await {
        return result;
    }
    task.abort();
    match task.await {
        Ok(()) => Ok(()),
        Err(error) if error.is_cancelled() => Ok(()),
        Err(error) => Err(error),
    }
}

impl Drop for LocalApiServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(crate) fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_binding(data_dir: PathBuf) -> Result<LocalApiServerBinding, LocalApiServerError> {
    let pid = std::process::id();
    let started = super::process_start_time(pid).ok_or_else(|| {
        LocalApiServerError::Discovery(io::Error::other("process identity unavailable"))
    })?;
    LocalApiServerBinding::bind(
        data_dir,
        uuid::Uuid::new_v4().simple().to_string(),
        pid,
        started,
        Arc::<str>::from(generate_token()).into(),
    )
}

#[cfg(test)]
#[path = "server_test.rs"]
mod server_tests;
