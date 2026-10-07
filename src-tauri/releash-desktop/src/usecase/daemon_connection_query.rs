use releashd::desktop_api::DesktopSettingsDto;
pub trait DaemonConnectionQueryService: Send + Sync {
    fn initial_settings(&self) -> Option<DesktopSettingsDto>;
    fn settings(&self) -> Option<DesktopSettingsDto>;
}
