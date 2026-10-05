mod conversions;
pub(crate) mod descriptor;
mod errors;
mod workspace;
pub(crate) fn value<T, U: TryFrom<T>>(value: T) -> Result<U, CommandFailure>
where
    U::Error: std::fmt::Display,
{
    U::try_from(value).map_err(|error| {
        crate::adaptor::presenter::error::AppError::new(error.to_string())
            .with_code("INVALID_RESPONSE")
            .into()
    })
}

pub(crate) fn outcome<T, U: TryFrom<T>, E: Into<CommandFailure>>(
    result: Result<T, E>,
) -> Result<U, CommandFailure>
where
    U::Error: std::fmt::Display,
{
    result.map_err(Into::into).and_then(value)
}

pub use errors::CommandFailure;
#[cfg(any(test, debug_assertions))]
mod json;
mod workflow_values;

#[cfg(any(test, debug_assertions))]
pub(crate) use self::json::from_message;
#[cfg(any(test, debug_assertions))]
use self::json::to_message;
#[cfg(any(test, debug_assertions))]
use serde_json::Value as Json;

include!(concat!(env!("OUT_DIR"), "/releash.client.v1.rs"));
include!(concat!(env!("OUT_DIR"), "/client_commands.rs"));

#[cfg(any(test, debug_assertions))]
pub trait ClientValue {
    fn into_json(self) -> Result<Json, String>;
}
#[cfg(any(test, debug_assertions))]
impl<T: ClientValue> ClientValue for Box<T> {
    fn into_json(self) -> Result<Json, String> {
        (*self).into_json()
    }
}
#[cfg(any(test, debug_assertions))]
impl ClientValue for CommandResult {
    fn into_json(self) -> Result<Json, String> {
        self.into_value().map(|(_, value)| value)
    }
}
#[cfg(any(test, debug_assertions))]
impl ClientValue for CommandError {
    fn into_json(self) -> Result<Json, String> {
        from_message("releash.client.v1.CommandError", &self)
    }
}
#[cfg(any(test, debug_assertions))]
pub fn from_value(value: impl ClientValue) -> Result<Json, String> {
    value.into_json()
}

impl From<crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1> for TerminalEvent {
    fn from(value: crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1) -> Self {
        use crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1 as Item;
        use terminal_event::Item as Wire;
        Self {
            item: Some(match value {
                Item::Snapshot { surface } => Wire::Snapshot(TerminalSnapshot {
                    session_key: surface.session_key,
                    processed_report_units:
                        crate::infrastructure::terminal::output_flow_control::OUTPUT_REPORT_UNITS
                            as u32,
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
            close_to_tray: Some(value.close_to_tray),
            start_minimized: Some(value.start_minimized),
            crash_reporting: Some(value.crash_reporting),
            performance_telemetry: Some(value.performance_telemetry),
            auto_launch: Some(value.auto_launch),
        }
    }
}

impl TryFrom<DesktopSettings> for crate::usecase::app_config::query_service::DesktopSettingsDto {
    type Error = String;
    fn try_from(value: DesktopSettings) -> Result<Self, String> {
        let field = |value: Option<bool>, name: &str| {
            value.ok_or_else(|| format!("Missing DesktopSettings.{name}"))
        };
        Ok(Self {
            close_to_tray: field(value.close_to_tray, "close_to_tray")?,
            start_minimized: field(value.start_minimized, "start_minimized")?,
            crash_reporting: field(value.crash_reporting, "crash_reporting")?,
            performance_telemetry: field(value.performance_telemetry, "performance_telemetry")?,
            auto_launch: field(value.auto_launch, "auto_launch")?,
        })
    }
}
