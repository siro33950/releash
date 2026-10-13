#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalShellState {
    pub cwd: Option<String>,
    pub has_foreground_process: bool,
}

impl TerminalShellState {
    pub fn has_foreground_process(
        shell_group: i32,
        foreground_group: i32,
        initial_program: &str,
        current_program: &str,
        child_groups: &[i32],
    ) -> bool {
        foreground_group > 0
            && (foreground_group != shell_group
                || initial_program != current_program
                || child_groups.contains(&foreground_group))
    }
}

#[cfg(test)]
#[path = "shell_state_test.rs"]
mod shell_state_tests;
