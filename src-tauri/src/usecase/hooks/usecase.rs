use crate::domain::hooks::services::merge_hooks;

pub fn apply_hooks_config(
    mut existing: serde_json::Value,
    new_config: serde_json::Value,
) -> Result<serde_json::Value, String> {
    merge_hooks(&mut existing, &new_config)?;
    Ok(existing)
}
