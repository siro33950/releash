use serde_json::Value;

use super::contract_schema::SchemaViolation;

pub const CONTRACT_NAME: &str = "spec-directory";
const FIELD_NAME: &str = "spec_dir";

pub fn validate_contract_value(contract: &str, value: &Value) -> Vec<SchemaViolation> {
    if contract != CONTRACT_NAME {
        return Vec::new();
    }
    value
        .get(FIELD_NAME)
        .and_then(Value::as_str)
        .and_then(|spec_dir| validate_spec_dir_path(spec_dir).err())
        .map(|reason| {
            vec![SchemaViolation {
                path: format!("$.{FIELD_NAME}"),
                reason: reason.to_string(),
            }]
        })
        .unwrap_or_default()
}

fn validate_spec_dir_path(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() {
        return Err("spec_dir must be a non-empty relative path");
    }
    if value.starts_with('/') || value.starts_with('\\') {
        return Err("spec_dir must be a relative path");
    }
    if has_windows_prefix(value) {
        return Err("spec_dir must not use a Windows drive or prefix");
    }
    if value.ends_with('/') || value.ends_with('\\') {
        return Err("spec_dir must not end with a path separator");
    }
    if value
        .split(['/', '\\'])
        .any(|component| component.is_empty() || component == "..")
    {
        return Err("spec_dir must not contain empty or parent path components");
    }
    Ok(())
}

fn has_windows_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(
        bytes,
        [drive, b':', ..] if drive.is_ascii_alphabetic()
    ) || value.starts_with("//")
        || value.starts_with("\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_contract_value_only_applies_to_spec_directory_contract() {
        assert!(
            validate_contract_value("custom", &serde_json::json!({"spec_dir": "../outside"}))
                .is_empty()
        );
        assert_eq!(
            validate_contract_value(
                CONTRACT_NAME,
                &serde_json::json!({"spec_dir": "../outside"})
            )[0]
            .path,
            "$.spec_dir"
        );
    }
}
