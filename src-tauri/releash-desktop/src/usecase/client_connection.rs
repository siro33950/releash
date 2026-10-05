pub(crate) use releash_lib::desktop_api::{ClientConnectionDto, ClientConnectionError};
pub(crate) trait ClientConnectionQueryService: Send + Sync {
    fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError>;
}
