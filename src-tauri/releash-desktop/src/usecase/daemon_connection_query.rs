use releashd::desktop_api::DesktopSettingsDto;
pub trait DaemonConnectionQueryService: Send + Sync {
    fn settings(&self) -> Option<DesktopSettingsDto>;
}
