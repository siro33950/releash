use crate::domain::daemon_connection::{DaemonEndpoint, DaemonResult};
use releashd::desktop_api::DesktopSettingsDto;

pub trait DaemonConnectionQueryService: Send + Sync {
    fn settings(&self) -> Option<DesktopSettingsDto>;
}
pub trait DesktopSettingsSubscription: DaemonConnectionQueryService {
    fn connect<'a>(&'a self, endpoint: &'a DaemonEndpoint) -> DaemonResult<'a, ()>;
}
