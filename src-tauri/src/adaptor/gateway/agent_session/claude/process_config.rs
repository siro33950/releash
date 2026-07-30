use std::path::PathBuf;
use std::time::Duration;

use crate::domain::agent_session::gateway::SessionSpec;
use crate::infrastructure::agent_session::claude::process::ClaudeProcessConfig;

use super::wire_conversion::claude_wire_mode;

pub(crate) fn build_process_config(
    cli_path: impl Into<String>,
    spec: &SessionSpec,
) -> ClaudeProcessConfig {
    ClaudeProcessConfig {
        cli_path: cli_path.into(),
        cwd: PathBuf::from(&spec.cwd),
        args: build_args(spec),
        env: watchdog_env(spec.stale_timeout),
        session_id: spec.session_id.clone(),
        base_branch: spec.base_branch.clone(),
        extra_env: spec.extra_env.clone(),
        system_prompt: spec.system_prompt.clone(),
    }
}

pub(crate) fn build_args(spec: &SessionSpec) -> Vec<String> {
    let mut args = vec![
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
        "--permission-prompt-tool".to_string(),
        "stdio".to_string(),
        "--allow-dangerously-skip-permissions".to_string(),
        "--setting-sources".to_string(),
        "user,project".to_string(),
        "--permission-mode".to_string(),
        claude_wire_mode(spec.permission_mode, spec.plan_mode)
            .as_str()
            .to_string(),
        "--model".to_string(),
        spec.model.as_str().to_string(),
    ];
    if let Some(resume) = spec.resume.as_deref().filter(|value| !value.is_empty()) {
        args.push("--resume".to_string());
        args.push(resume.to_string());
    }
    args
}

pub(crate) fn watchdog_env(stale_timeout: Option<Duration>) -> Vec<(String, String)> {
    let mut env = vec![
        ("CLAUDE_CODE_MAX_RETRIES".to_string(), "10".to_string()),
        ("API_TIMEOUT_MS".to_string(), "600000".to_string()),
    ];
    if let Some(stale_timeout) = stale_timeout {
        env.extend([
            ("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "1".to_string()),
            ("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "1".to_string()),
            (
                "CLAUDE_STREAM_IDLE_TIMEOUT_MS".to_string(),
                stale_timeout.as_millis().to_string(),
            ),
        ]);
    } else {
        env.extend([
            ("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "0".to_string()),
            ("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "0".to_string()),
        ]);
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_session::value_objects::{ModelId, PermissionMode};

    fn spec() -> SessionSpec {
        SessionSpec {
            session_id: "s1".to_string(),
            cwd: "/repo".to_string(),
            permission_mode: PermissionMode::Edit,
            plan_mode: false,
            permission_profile_id: None,
            model: ModelId::parse("claude-sonnet-4-5").unwrap(),
            system_prompt: Some("system".to_string()),
            resume: Some("backend-session".to_string()),
            base_branch: Some("main".to_string()),
            startup_timeout: None,
            startup_max_retries: None,
            stale_timeout: Some(Duration::from_secs(42)),
            extra_env: Vec::new(),
        }
    }

    #[test]
    fn required_provider_flags_are_built_at_the_gateway_boundary() {
        let args = build_args(&spec());

        assert!(args.contains(&"--input-format".to_string()));
        assert!(args.contains(&"--include-partial-messages".to_string()));
        assert!(args.contains(&"--permission-prompt-tool".to_string()));
        assert!(args.contains(&"stdio".to_string()));
        assert!(args.contains(&"--allow-dangerously-skip-permissions".to_string()));
        assert!(args.contains(&"--resume".to_string()));
        assert!(args.contains(&"backend-session".to_string()));
    }

    #[test]
    fn stale_timeout_is_mapped_to_watchdog_environment() {
        let env = watchdog_env(Some(Duration::from_secs(42)));
        assert!(env.contains(&(
            "CLAUDE_STREAM_IDLE_TIMEOUT_MS".to_string(),
            "42000".to_string()
        )));
        assert!(env.contains(&("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "1".to_string())));
        assert!(env.contains(&("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "1".to_string())));
        assert!(env.contains(&("CLAUDE_CODE_MAX_RETRIES".to_string(), "10".to_string())));
    }

    #[test]
    fn missing_stale_timeout_disables_provider_watchdogs() {
        let env = watchdog_env(None);
        assert!(!env
            .iter()
            .any(|(key, _)| key == "CLAUDE_STREAM_IDLE_TIMEOUT_MS"));
        assert!(env.contains(&("CLAUDE_ENABLE_STREAM_WATCHDOG".to_string(), "0".to_string())));
        assert!(env.contains(&("CLAUDE_ENABLE_BYTE_WATCHDOG".to_string(), "0".to_string())));
    }
}
