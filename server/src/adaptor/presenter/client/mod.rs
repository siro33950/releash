mod conversions;
pub(crate) use conversions::session_selection;
pub(crate) mod descriptor;
pub(crate) mod errors;
mod workspace;
pub(crate) use workspace::{branch_status, selection};
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
#[cfg(any(test, feature = "test-support"))]
mod json;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use self::json::from_message;
#[cfg(any(test, feature = "test-support"))]
use self::json::to_message;
#[cfg(any(test, feature = "test-support"))]
use serde_json::Value as Json;

pub use crate::infrastructure::client_protocol::wire::*;
mod values;
include!(concat!(env!("OUT_DIR"), "/client_commands.rs"));

#[cfg(any(test, feature = "test-support"))]
pub trait ClientValue {
    fn into_json(self) -> Result<Json, String>;
}
#[cfg(any(test, feature = "test-support"))]
impl<T: ClientValue> ClientValue for Box<T> {
    fn into_json(self) -> Result<Json, String> {
        (*self).into_json()
    }
}
#[cfg(any(test, feature = "test-support"))]
impl ClientValue for CommandResult {
    fn into_json(self) -> Result<Json, String> {
        self.into_value().map(|(_, value)| value)
    }
}
#[cfg(any(test, feature = "test-support"))]
impl ClientValue for CommandError {
    fn into_json(self) -> Result<Json, String> {
        from_message("releash.client.v1.CommandError", &self)
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn from_value(value: impl ClientValue) -> Result<Json, String> {
    value.into_json()
}

fn terminal_status(
    session_key: String,
    state: crate::adaptor::presenter::terminal::TerminalShellStateV1,
    sequence: u64,
) -> TerminalStatus {
    TerminalStatus {
        session_key,
        cwd: state.cwd,
        has_foreground_process: state.has_foreground_process,
        sequence,
    }
}

impl From<crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1> for TerminalEvent {
    fn from(value: crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1) -> Self {
        use crate::adaptor::presenter::terminal::TerminalSurfaceStreamItemV1 as Item;
        use terminal_event::Item as Wire;
        Self {
            item: Some(match value {
                Item::Status {
                    session_key,
                    state,
                    sequence,
                } => Wire::Status(terminal_status(session_key, state, sequence)),
                Item::Snapshot { surface } => Wire::Snapshot(TerminalSnapshot {
                    status: surface.shell_state.map(|state| {
                        terminal_status(
                            surface.session_key.clone(),
                            state,
                            surface.terminal_surface.sequence,
                        )
                    }),
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
                    state,
                    session_key,
                    exit_code,
                    sequence,
                } => Wire::Exit(TerminalExit {
                    status: state
                        .map(|state| terminal_status(session_key.clone(), state, sequence)),
                    session_key,
                    exit_code,
                    sequence,
                }),
            }),
        }
    }
}

impl From<&crate::domain::app_config::AppConfig> for DesktopSettings {
    fn from(value: &crate::domain::app_config::AppConfig) -> Self {
        Self {
            start_minimized: Some(value.start_minimized()),
            crash_reporting: Some(value.telemetry().crash_reporting),
            performance_telemetry: Some(value.telemetry().performance_telemetry),
            auto_launch: Some(value.auto_launch()),
        }
    }
}

#[cfg(feature = "test-support")]
#[path = "test_helpers.rs"]
pub(crate) mod test_helpers;

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_tests;
