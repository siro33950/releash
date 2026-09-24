mod conversions;
mod errors;
pub(crate) use errors::CommandFailure;
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
mod json;
mod workflow_values;

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
use self::json::from_message;
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
use self::json::to_message;
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
use serde_json::Value as Json;

include!(concat!(env!("OUT_DIR"), "/releash.client.v1.rs"));
include!(concat!(env!("OUT_DIR"), "/client_commands.rs"));

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
pub trait ClientValue {
    fn into_json(self) -> Result<Json, String>;
}
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
impl<T: ClientValue> ClientValue for Box<T> {
    fn into_json(self) -> Result<Json, String> {
        (*self).into_json()
    }
}
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
impl ClientValue for CommandResult {
    fn into_json(self) -> Result<Json, String> {
        self.into_value().map(|(_, value)| value)
    }
}
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
impl ClientValue for CommandError {
    fn into_json(self) -> Result<Json, String> {
        from_message("releash.client.v1.CommandError", &self)
    }
}
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
pub fn from_value(value: impl ClientValue) -> Result<Json, String> {
    value.into_json()
}

impl From<crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1> for TerminalEvent {
    fn from(value: crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1) -> Self {
        use crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1 as Item;
        use terminal_event::Item as Wire;
        Self {
            item: Some(match value {
                Item::Snapshot { surface } => Wire::Snapshot(TerminalSnapshot {
                    session_key: surface.session_key,
                    replay: surface.terminal_surface.replay,
                    sequence: surface.terminal_surface.sequence,
                    cols: surface.terminal_surface.cols.into(),
                    rows: surface.terminal_surface.rows.into(),
                    is_exited: surface.is_exited,
                    exit_code: surface.exit_code,
                }),
                Item::Output {
                    session_key,
                    data,
                    sequence,
                } => Wire::Output(TerminalOutput {
                    session_key,
                    data: data.to_string(),
                    sequence,
                }),
                Item::Resize {
                    session_key,
                    cols,
                    rows,
                    sequence,
                } => Wire::Resize(TerminalResize {
                    session_key,
                    cols: cols.into(),
                    rows: rows.into(),
                    sequence,
                }),
                Item::Exit {
                    session_key,
                    exit_code,
                    sequence,
                } => Wire::Exit(TerminalExit {
                    session_key,
                    exit_code,
                    sequence,
                }),
                Item::InputUnavailable {
                    session_key,
                    message,
                } => Wire::InputUnavailable(TerminalInputUnavailable {
                    session_key,
                    message,
                }),
            }),
        }
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_tests;

impl From<crate::usecase::app_config::query_service::DesktopSettingsDto> for DesktopSettings {
    fn from(value: crate::usecase::app_config::query_service::DesktopSettingsDto) -> Self {
        Self {
            close_to_tray: value.close_to_tray,
            start_minimized: value.start_minimized,
            crash_reporting: value.crash_reporting,
            performance_telemetry: value.performance_telemetry,
        }
    }
}

#[cfg(feature = "desktop")]
impl From<DesktopSettings> for crate::usecase::app_config::query_service::DesktopSettingsDto {
    fn from(value: DesktopSettings) -> Self {
        Self {
            close_to_tray: value.close_to_tray,
            start_minimized: value.start_minimized,
            crash_reporting: value.crash_reporting,
            performance_telemetry: value.performance_telemetry,
        }
    }
}
