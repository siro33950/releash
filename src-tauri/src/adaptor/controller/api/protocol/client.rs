use super::json::from_message;
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
use super::json::to_message;
use prost::Message;
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
use serde_json::Value as Json;

include!(concat!(env!("OUT_DIR"), "/releash.client.v1.rs"));
include!(concat!(env!("OUT_DIR"), "/client_commands.rs"));

pub const MAX_STREAM_FRAME_BYTES: usize = 64 * 1024;

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

pub(crate) fn response(
    request_id: String,
    result: Result<command_result::Command, CommandError>,
) -> Envelope {
    let outcome = match result {
        Ok(command) => command_response::Outcome::Result(Box::new(CommandResult {
            command: Some(command),
        })),
        Err(error) => command_response::Outcome::Error(error),
    };
    Envelope {
        body: Some(envelope::Body::Response(CommandResponse {
            request_id,
            outcome: Some(outcome),
            desktop_settings: None,
        })),
    }
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

pub(crate) fn stream_frame(
    attachment_id: &str,
    sequence: u64,
    data: Vec<u8>,
    end: bool,
) -> Vec<u8> {
    Envelope {
        body: Some(envelope::Body::Stream(Stream {
            attachment_id: attachment_id.into(),
            sequence,
            data,
            end,
        })),
    }
    .encode_to_vec()
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
