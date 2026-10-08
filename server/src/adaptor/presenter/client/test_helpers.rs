use super::{command_request, CommandRequest};
use super::{CommandName, CommandRequestEncode};

pub fn command_request_from_value(
    name: &str,
    value: serde_json::Value,
) -> Result<CommandRequest, String> {
    CommandRequest::from_value(name, value)
}

pub fn command_name(command: &command_request::Command) -> &'static str {
    command.name()
}
