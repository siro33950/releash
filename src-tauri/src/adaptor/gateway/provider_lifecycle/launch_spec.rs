use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::agent_session::ProviderSessionLaunch;
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId,
};
use crate::domain::terminal_surface::TerminalProcessLaunch;
use crate::domain::workflow::SessionPermission;

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ProviderLaunchSpecError {
    #[error("Provider launch field is empty: {0}")]
    EmptyField(&'static str),
    #[error("Claude plugin directory is required")]
    ClaudePluginDirectoryRequired,
    #[error("Unsupported managed Releash CLI alias")]
    UnsupportedCliAlias,
    #[error("Generated Provider launch is invalid")]
    InvalidGeneratedLaunch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLaunchContext {
    slot_id: ProviderLifecycleSlotId,
    binding_id: String,
    capability: String,
    scope: ProviderLifecycleScope,
}

impl ProviderLaunchContext {
    pub(crate) fn new(
        slot_id: ProviderLifecycleSlotId,
        binding_id: impl Into<String>,
        capability: impl Into<String>,
        scope: ProviderLifecycleScope,
    ) -> Result<Self, ProviderLaunchSpecError> {
        let binding_id = binding_id.into();
        let capability = capability.into();
        if binding_id.trim().is_empty() {
            return Err(ProviderLaunchSpecError::EmptyField("binding_id"));
        }
        if capability.trim().is_empty() {
            return Err(ProviderLaunchSpecError::EmptyField("capability"));
        }
        Ok(Self {
            slot_id,
            binding_id,
            capability,
            scope,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLaunchFile {
    relative_path: PathBuf,
    contents: Vec<u8>,
}

impl ProviderLaunchFile {
    pub(crate) fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub(crate) fn contents(&self) -> &[u8] {
        &self.contents
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderLaunchSpec {
    provider: ProviderKind,
    arguments: Vec<String>,
    environment: Vec<(String, String)>,
    files: Vec<ProviderLaunchFile>,
    requires_hook_trust: bool,
}

impl ProviderLaunchSpec {
    pub(crate) fn for_provider(
        provider: ProviderKind,
        context: ProviderLaunchContext,
        hook_cli_alias: &str,
        claude_plugin_directory: Option<&Path>,
    ) -> Result<Self, ProviderLaunchSpecError> {
        if !matches!(hook_cli_alias, "releash" | "releash-dev") {
            return Err(ProviderLaunchSpecError::UnsupportedCliAlias);
        }
        let hook_command =
            |provider: &str| format!("{hook_cli_alias} hook receive --provider {provider}");
        let environment = vec![
            (
                "RELEASH_PROVIDER_LIFECYCLE_SLOT_ID".to_string(),
                context.slot_id.as_str().to_string(),
            ),
            (
                "RELEASH_PROVIDER_LIFECYCLE_BINDING_ID".to_string(),
                context.binding_id,
            ),
            (
                "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY".to_string(),
                context.capability,
            ),
            (
                "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID".to_string(),
                context.scope.agent_session_id().to_string(),
            ),
        ];

        match provider {
            ProviderKind::Claude => {
                let plugin_directory = claude_plugin_directory
                    .filter(|path| !path.as_os_str().is_empty())
                    .ok_or(ProviderLaunchSpecError::ClaudePluginDirectoryRequired)?;
                Ok(Self {
                    provider,
                    arguments: vec![
                        "--plugin-dir".to_string(),
                        plugin_directory.to_string_lossy().into_owned(),
                    ],
                    environment,
                    files: claude_plugin_files(&hook_command("claude")),
                    requires_hook_trust: false,
                })
            }
            ProviderKind::Codex => {
                let command = hook_command("codex");
                Ok(Self {
                    provider,
                    arguments: vec![
                        "-c".to_string(),
                        format!(
                            "hooks.SessionStart=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                        "-c".to_string(),
                        format!(
                            "hooks.Stop=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                        "-c".to_string(),
                        format!(
                            "hooks.UserPromptSubmit=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                        "-c".to_string(),
                        format!(
                            "hooks.PreToolUse=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                        "-c".to_string(),
                        format!(
                            "hooks.PostToolUse=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                        "-c".to_string(),
                        format!(
                            "hooks.PermissionRequest=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                    ],
                    environment,
                    files: Vec::new(),
                    requires_hook_trust: true,
                })
            }
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn arguments(&self) -> &[String] {
        &self.arguments
    }

    #[cfg(debug_assertions)]
    pub(crate) fn environment(&self) -> &[(String, String)] {
        &self.environment
    }

    pub(crate) fn files(&self) -> &[ProviderLaunchFile] {
        &self.files
    }

    pub(crate) fn requires_hook_trust(&self) -> bool {
        self.requires_hook_trust
    }

    pub(crate) fn terminal_process(
        &self,
        executable: impl Into<std::ffi::OsString>,
        launch: ProviderSessionLaunch,
    ) -> Result<TerminalProcessLaunch, ProviderLaunchSpecError> {
        let executable = executable.into();
        if executable.is_empty() {
            return Err(ProviderLaunchSpecError::EmptyField("executable"));
        }
        let mut arguments = self.arguments.clone();
        if let Some(model) = launch.options().model.as_deref() {
            arguments.extend(["--model".to_string(), model.to_string()]);
        }
        if let Some(permission) = launch.options().permission {
            let flags: &'static [&'static str] = match (self.provider, permission) {
                (ProviderKind::Claude, SessionPermission::Manual) => {
                    &["--permission-mode", "default"]
                }
                (ProviderKind::Claude, SessionPermission::Auto) => &["--permission-mode", "auto"],
                (ProviderKind::Claude, SessionPermission::Bypass) => {
                    &["--permission-mode", "bypassPermissions"]
                }
                (ProviderKind::Claude, SessionPermission::ReadOnly) => {
                    &["--permission-mode", "plan"]
                }
                (ProviderKind::Codex, SessionPermission::Manual) => &[
                    "--sandbox",
                    "workspace-write",
                    "--ask-for-approval",
                    "on-request",
                ],
                (ProviderKind::Codex, SessionPermission::Auto) => &["--approve-for-me"],
                (ProviderKind::Codex, SessionPermission::Bypass) => {
                    &["--dangerously-bypass-approvals-and-sandbox"]
                }
                (ProviderKind::Codex, SessionPermission::ReadOnly) => {
                    &["--sandbox", "read-only", "--ask-for-approval", "never"]
                }
            };
            arguments.extend(flags.iter().map(|flag| flag.to_string()));
        }
        if let Some(provider_session_id) = launch.provider_session_id() {
            match self.provider {
                ProviderKind::Claude => {
                    arguments.extend(["--resume".to_string(), provider_session_id.to_string()])
                }
                ProviderKind::Codex => {
                    arguments.extend(["resume".to_string(), provider_session_id.to_string()])
                }
            }
        }
        if let Some(initial_instruction) = launch.initial_instruction() {
            arguments.push(initial_instruction.to_string());
        }
        TerminalProcessLaunch::new(executable, arguments, self.environment.clone())
            .map_err(|_| ProviderLaunchSpecError::InvalidGeneratedLaunch)
    }
}

fn claude_plugin_files(hook_command: &str) -> Vec<ProviderLaunchFile> {
    let hooks = serde_json::json!({
        "hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "Stop": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "StopFailure": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "UserPromptSubmit": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "PreToolUse": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "PostToolUse": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "PermissionRequest": [{"hooks": [{"type": "command", "command": hook_command}]}]
        }
    })
    .to_string()
    .into_bytes();
    vec![
        ProviderLaunchFile {
            relative_path: PathBuf::from(".claude-plugin/plugin.json"),
            contents: br#"{
                "name":"releash-provider-lifecycle",
                "version":"1.0.0",
                "description":"Releash Provider lifecycle integration",
                "author":{"name":"Releash"}
            }"#
            .to_vec(),
        },
        ProviderLaunchFile {
            relative_path: PathBuf::from("hooks/hooks.json"),
            contents: hooks,
        },
    ]
}
