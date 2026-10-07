use releashd::desktop_api::DesktopSettingsDto;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionFailure {
    pub message: String,
    pub server_older: bool,
}

pub trait DaemonConnectionQueryService: Send + Sync {
    fn failure(&self) -> Option<ConnectionFailure>;
    fn settings(&self) -> Option<DesktopSettingsDto>;
    fn settings_update(&self) -> Option<DesktopSettingsDto>;
}
