//! Pure secret masking for workflow logs, comments, and structured outputs.

use std::sync::LazyLock;

static PRIVATE_KEY_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?is)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----")
        .unwrap()
});
static GHP_TOKEN_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\bghp_[A-Za-z0-9_]{20,}\b").unwrap());
static GITHUB_PAT_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b").unwrap());
static SECRET_KV_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(api_key|apikey|token|password|secret)\s*[:=]\s*([^\s,;]+)").unwrap()
});

pub fn mask_sensitive_text(text: &str, configured_secrets: &[String]) -> String {
    let mut masked = text.to_string();
    masked = PRIVATE_KEY_RE
        .replace_all(&masked, "[REDACTED]")
        .into_owned();
    masked = GHP_TOKEN_RE.replace_all(&masked, "[REDACTED]").into_owned();
    masked = GITHUB_PAT_RE
        .replace_all(&masked, "[REDACTED]")
        .into_owned();
    masked = SECRET_KV_RE
        .replace_all(&masked, "$1=[REDACTED]")
        .into_owned();
    for secret in configured_secrets {
        if !secret.is_empty() {
            masked = masked.replace(secret.as_str(), "[REDACTED]");
        }
    }
    masked
}

pub fn mask_json_strings(value: &mut serde_json::Value, configured_secrets: &[String]) {
    match value {
        serde_json::Value::String(s) => {
            *s = mask_sensitive_text(s, configured_secrets);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                mask_json_strings(item, configured_secrets);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                mask_json_strings(item, configured_secrets);
            }
        }
        _ => {}
    }
}

pub fn mask_sensitive_artifact(
    _contract: &str,
    mut value: serde_json::Value,
    secrets: &[String],
) -> serde_json::Value {
    mask_json_strings(&mut value, secrets);
    value
}

pub fn collect_secret_values_from_env_vars<I>(vars: I) -> Vec<String>
where
    I: IntoIterator<Item = (String, String)>,
{
    vars.into_iter()
        .filter_map(|(key, value)| {
            if value.len() >= 8 && is_secret_env_var_name(&key) {
                Some(value)
            } else {
                None
            }
        })
        .collect()
}

pub fn normalize_secret_values(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    values
}

fn is_secret_env_var_name(name: &str) -> bool {
    let normalized = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "API_KEY",
        "ACCESS_KEY",
        "PRIVATE_KEY",
        "CREDENTIAL",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
}

#[cfg(test)]
#[path = "secret_masker_test.rs"]
mod secret_masker_tests;
