use crate::domain::hooks::services::{build_hooks_json, detect_hooks_status};
use crate::domain::hooks::value_objects::HooksStatus;

pub fn generate_hooks_config(port: u16, token: &str) -> Result<String, String> {
    serde_json::to_string_pretty(&build_hooks_json(port, token))
        .map_err(|e| format!("JSON生成失敗: {e}"))
}

pub fn get_hooks_status(
    settings: Option<serde_json::Value>,
    hook_port: u16,
    token: &str,
) -> HooksStatus {
    match settings {
        Some(settings) => detect_hooks_status(&settings, hook_port, token),
        None => HooksStatus::NotConfigured,
    }
}
