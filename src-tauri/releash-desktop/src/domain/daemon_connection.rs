#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub url: String,
    pub token: String,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ConnectionError {
    pub message: String,
    pub server_older: bool,
}

impl From<String> for ConnectionError {
    fn from(message: String) -> Self {
        Self {
            message,
            server_older: false,
        }
    }
}

#[async_trait::async_trait]
pub trait DaemonConnectionPort: Send + Sync {
    async fn discover(&self) -> Result<Option<DaemonEndpoint>, ConnectionError>;
    async fn start(&self) -> Result<(), ConnectionError>;
    async fn subscribe(&self, endpoint: &DaemonEndpoint) -> Result<(), ConnectionError>;
    async fn stop(&self) -> Result<(), ConnectionError>;
    fn subscribed_to(&self, endpoint: &DaemonEndpoint) -> bool;
    fn record_failure(&self, failure: Option<ConnectionError>);
}
