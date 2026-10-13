use crate::domain::terminal_surface::TerminalShellState;

#[derive(Default)]
pub struct TerminalShellStatus {
    osc7_cwd: Option<String>,
    process_cwd: Option<String>,
    latest: Option<TerminalShellState>,
}

impl TerminalShellStatus {
    pub fn latest(&self) -> Option<TerminalShellState> {
        self.latest.clone()
    }

    pub fn observe(
        &mut self,
        osc7_cwd: Option<String>,
        updated: bool,
        process_cwd: String,
        mut state: TerminalShellState,
    ) -> Option<TerminalShellState> {
        if updated {
            self.osc7_cwd = osc7_cwd;
        } else if self
            .process_cwd
            .as_ref()
            .is_some_and(|previous| previous != &process_cwd)
        {
            self.osc7_cwd = None;
        }
        self.process_cwd = Some(process_cwd.clone());
        state.cwd = Some(self.osc7_cwd.clone().unwrap_or(process_cwd));
        if self.latest.as_ref() == Some(&state) {
            return None;
        }
        self.latest = Some(state.clone());
        Some(state)
    }
}

#[cfg(test)]
#[path = "terminal_shell_status_test.rs"]
mod terminal_shell_status_tests;
