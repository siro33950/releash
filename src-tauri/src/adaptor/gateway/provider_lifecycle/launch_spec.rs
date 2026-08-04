use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ProviderLaunchSpecError {
    #[error("Provider launch field is empty: {0}")]
    EmptyField(&'static str),
    #[error("Claude plugin directory is required")]
    ClaudePluginDirectoryRequired,
    #[error("Unsupported managed Releash CLI alias")]
    UnsupportedCliAlias,
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
            (
                "RELEASH_PROVIDER_LIFECYCLE_WORKFLOW_EXECUTION_ID".to_string(),
                context.scope.workflow_execution_id().to_string(),
            ),
            (
                "RELEASH_PROVIDER_LIFECYCLE_NODE_EXECUTION_ID".to_string(),
                context.scope.node_execution_id().to_string(),
            ),
            (
                "RELEASH_PROVIDER_LIFECYCLE_ATTEMPT".to_string(),
                context.scope.attempt().to_string(),
            ),
        ];

        match provider {
            ProviderKind::Claude => {
                let plugin_directory = claude_plugin_directory
                    .filter(|path| !path.as_os_str().is_empty())
                    .ok_or(ProviderLaunchSpecError::ClaudePluginDirectoryRequired)?;
                Ok(Self {
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
                    arguments: vec![
                        "-c".to_string(),
                        format!(
                            "hooks.SessionStart=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                        "-c".to_string(),
                        format!(
                            "hooks.Stop=[{{hooks=[{{type=\"command\",command=\"{command}\"}}]}}]"
                        ),
                    ],
                    environment,
                    files: Vec::new(),
                    requires_hook_trust: true,
                })
            }
        }
    }

    pub(crate) fn arguments(&self) -> &[String] {
        &self.arguments
    }

    pub(crate) fn environment(&self) -> &[(String, String)] {
        &self.environment
    }

    pub(crate) fn files(&self) -> &[ProviderLaunchFile] {
        &self.files
    }

    pub(crate) fn requires_hook_trust(&self) -> bool {
        self.requires_hook_trust
    }
}

fn claude_plugin_files(hook_command: &str) -> Vec<ProviderLaunchFile> {
    let hooks = serde_json::json!({
        "hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "Stop": [{"hooks": [{"type": "command", "command": hook_command}]}],
            "StopFailure": [{"hooks": [{"type": "command", "command": hook_command}]}]
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
