use std::collections::HashSet;
use std::ffi::{OsStr, OsString};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalProcessLaunch {
    executable: OsString,
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
        executable: impl Into<OsString>,
        arguments: Vec<String>,
        environment: Vec<(String, String)>,
    ) -> Result<Self, TerminalProcessLaunchError> {
        let executable = executable.into();
        if executable.is_empty()
            || executable
                .to_str()
                .is_some_and(|value| value.trim().is_empty())
        {
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

    pub fn executable(&self) -> &OsStr {
        &self.executable
    }

    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub fn environment(&self) -> &[(String, String)] {
        &self.environment
    }
}
