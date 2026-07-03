use tokio::process::Command;

use crate::infrastructure::platform::path_aliases::{child_env_overrides, PathAliases};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentChildEnv {
    envs: Vec<(String, String)>,
    scrub_envs: Vec<String>,
}

impl AgentChildEnv {
    pub(crate) fn for_session(
        session_id: &str,
        base_branch: Option<&str>,
        extra_env: impl IntoIterator<Item = (String, String)>,
        scrub_envs: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        let mut envs = Vec::new();
        if let Ok(aliases) = PathAliases::from_runtime(None) {
            match child_env_overrides(&aliases) {
                Ok(overrides) => envs.extend(overrides),
                Err(error) => log::warn!("failed to build Releash child env overrides: {error}"),
            }
        }
        envs.push(("RELEASH_SESSION_ID".to_string(), session_id.to_string()));
        if let Some(base_branch) = base_branch.filter(|value| !value.trim().is_empty()) {
            envs.push(("RELEASH_BASE_BRANCH".to_string(), base_branch.to_string()));
        }
        envs.extend(extra_env);
        Self {
            envs,
            scrub_envs: scrub_envs.into_iter().map(str::to_string).collect(),
        }
    }

    pub(crate) fn apply(&self, command: &mut Command) {
        for key in &self.scrub_envs {
            command.env_remove(key);
        }
        command.envs(self.envs.clone());
    }

    #[cfg(test)]
    pub(crate) fn envs(&self) -> &[(String, String)] {
        &self.envs
    }

    #[cfg(test)]
    pub(crate) fn scrub_envs(&self) -> &[String] {
        &self.scrub_envs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_child_env_session_idとbase_branchを含む() {
        let env = AgentChildEnv::for_session(
            "session-1",
            Some("main"),
            [("EXTRA".to_string(), "1".to_string())],
            ["REMOVE_ME"],
        );

        assert!(env
            .envs()
            .contains(&("RELEASH_SESSION_ID".to_string(), "session-1".to_string())));
        assert!(env
            .envs()
            .contains(&("RELEASH_BASE_BRANCH".to_string(), "main".to_string())));
        assert!(env.envs().contains(&("EXTRA".to_string(), "1".to_string())));
        assert_eq!(env.scrub_envs(), &["REMOVE_ME".to_string()]);
    }
}
