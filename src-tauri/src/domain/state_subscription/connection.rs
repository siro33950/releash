use super::{StateValue, Version};
#[derive(Debug)]
pub(crate) struct StateClientError(pub String);

pub(crate) struct ReceivedState {
    pub target: String,
    pub version: Version,
    pub snapshot: bool,
    pub value: Option<StateValue>,
}

#[async_trait::async_trait]
pub(crate) trait StateConnection: Send {
    async fn start(
        &mut self,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), StateClientError>;
    async fn stop(&mut self, target: &str) -> Result<(), StateClientError>;
    async fn receive(&mut self) -> Result<ReceivedState, StateClientError>;
}

#[async_trait::async_trait]
pub(crate) trait StateClientGateway: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn StateConnection>, StateClientError>;
}

impl std::fmt::Display for StateClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for StateClientError {}
