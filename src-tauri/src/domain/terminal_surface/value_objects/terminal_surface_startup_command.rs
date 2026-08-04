#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceStartupCommand(String);

impl TerminalSurfaceStartupCommand {
    pub fn new(command: Option<&str>) -> Option<Self> {
        let command = command?.trim();
        if command.is_empty() {
            return None;
        }
        Some(Self(command.to_string()))
    }

    pub fn into_input(self, restored_from_checkpoint: bool) -> Option<String> {
        if restored_from_checkpoint {
            return None;
        }
        Some(format!("{}\n", self.0))
    }
}

#[cfg(test)]
#[path = "terminal_surface_startup_command_test.rs"]
mod terminal_surface_startup_command_tests;
