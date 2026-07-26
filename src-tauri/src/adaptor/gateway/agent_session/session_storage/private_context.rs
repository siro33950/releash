use std::io::BufReader;
use std::path::{Path, PathBuf};

use super::layout::private_context_file_in_dir;
#[cfg(test)]
use super::layout::write_json_pretty_atomic;
use crate::usecase::agent_session::context_meta::ContextSourcePayloadCache;
use crate::usecase::agent_session::session::SessionMeta;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionPrivateContext {
    #[serde(rename = "workflowInstruction", default, skip_serializing)]
    pub legacy_workflow_instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflow_instructions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_epoch_payloads: Vec<ContextSourcePayloadCache>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_read_paths: Option<Vec<PathBuf>>,
}

fn apply_private_context(private_context: SessionPrivateContext, meta: &mut SessionMeta) {
    meta.workflow_instructions = private_context.workflow_instructions;
    meta.agent_read_paths = private_context.agent_read_paths;
    if let Some(instruction) = private_context
        .legacy_workflow_instruction
        .filter(|instruction| !instruction.trim().is_empty())
    {
        meta.workflow_instructions.push(instruction);
    }
    if let Some(context_epoch) = meta.context_epoch.as_mut() {
        context_epoch.hydrate_payload_cache(&private_context.context_epoch_payloads);
    }
}

#[cfg(test)]
fn private_context_from_meta(meta: &SessionMeta) -> SessionPrivateContext {
    SessionPrivateContext {
        legacy_workflow_instruction: None,
        workflow_instructions: meta
            .workflow_instructions
            .iter()
            .filter(|instruction| !instruction.trim().is_empty())
            .cloned()
            .collect(),
        context_epoch_payloads: meta
            .context_epoch
            .as_ref()
            .map(|context_epoch| context_epoch.payload_cache_entries())
            .unwrap_or_default(),
        agent_read_paths: meta.agent_read_paths.clone(),
    }
}

pub(super) fn hydrate_meta_private_context(dir: &Path, meta: &mut SessionMeta) {
    let path = private_context_file_in_dir(dir);
    if !path.exists() {
        return;
    }
    let Ok(file) = std::fs::File::open(&path) else {
        log::warn!(
            "Failed to open session private context at {}",
            path.display()
        );
        return;
    };
    let private_context: SessionPrivateContext = match serde_json::from_reader(BufReader::new(file))
    {
        Ok(context) => context,
        Err(err) => {
            log::warn!(
                "Failed to read session private context at {}: {err}",
                path.display()
            );
            return;
        }
    };
    apply_private_context(private_context, meta);
}

#[cfg(test)]
pub(super) fn write_private_context_to_dir(dir: &Path, meta: &SessionMeta) -> Result<(), String> {
    let path = private_context_file_in_dir(dir);
    let private_context = private_context_from_meta(meta);
    if private_context.workflow_instructions.is_empty()
        && private_context.context_epoch_payloads.is_empty()
        && private_context.agent_read_paths.is_none()
    {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!("Failed to remove session private context: {err}"));
            }
        }
        return Ok(());
    }
    write_json_pretty_atomic(&path, &private_context, "session private context")
}
