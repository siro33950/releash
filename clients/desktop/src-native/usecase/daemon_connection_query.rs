/// daemon から受け取る、desktop シェルの起動と telemetry の設定値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettingsDto {
    pub start_minimized: bool,
    pub crash_reporting: bool,
    pub performance_telemetry: bool,
    pub auto_launch: bool,
}

pub trait DaemonConnectionQueryService: Send + Sync {
    fn initial_settings(&self) -> Option<DesktopSettingsDto>;
    fn settings(&self) -> Option<DesktopSettingsDto>;
}
