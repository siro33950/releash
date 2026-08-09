use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalProcessLaunch {
    executable: String,
    arguments: Vec<String>,
    environment: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalProcessLaunchError {
    ExecutableMissing,
    EnvironmentKeyMissing,
    EnvironmentKeyDuplicate,
}

impl TerminalProcessLaunch {
    pub fn new(
        executable: impl Into<String>,
        arguments: Vec<String>,
        environment: Vec<(String, String)>,
    ) -> Result<Self, TerminalProcessLaunchError> {
        let executable = executable.into();
        if executable.trim().is_empty() {
            return Err(TerminalProcessLaunchError::ExecutableMissing);
        }
        let mut keys = HashSet::new();
        for (key, _) in &environment {
            if key.trim().is_empty() {
                return Err(TerminalProcessLaunchError::EnvironmentKeyMissing);
            }
            if !keys.insert(key) {
                return Err(TerminalProcessLaunchError::EnvironmentKeyDuplicate);
            }
        }
        Ok(Self {
            executable,
            arguments,
            environment,
        })
    }

    pub fn executable(&self) -> &str {
        &self.executable
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub fn environment(&self) -> &[(String, String)] {
        &self.environment
    }
}
