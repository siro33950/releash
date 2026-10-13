use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::provider_usage::{
    ProviderUsage, ProviderUsageOutput, ProviderUsageQueryService, ProviderUsageSource, UsageError,
    UsageWindow,
};
use std::path::PathBuf;

#[derive(Default)]
pub struct ProviderUsageReadModel(parking_lot::RwLock<Vec<ProviderUsage>>);
impl ProviderUsageQueryService for ProviderUsageReadModel {
    fn snapshot(&self) -> Vec<ProviderUsage> {
        self.0.read().clone()
    }
}
impl ProviderUsageOutput for ProviderUsageReadModel {
    fn replace(&self, usage: ProviderUsage) {
        let mut values = self.0.write();
        if let Some(value) = values
            .iter_mut()
            .find(|value| value.provider == usage.provider)
        {
            *value = usage;
        } else {
            values.push(usage);
        }
    }
}

pub struct LocalProviderUsageSource {
    pub claude_account: PathBuf,
    pub codex_home: PathBuf,
}

#[async_trait::async_trait]
impl ProviderUsageSource for LocalProviderUsageSource {
    fn claude(&self, payload: &[u8]) -> Result<Option<ProviderUsage>, UsageError> {
        let value: serde_json::Value =
            serde_json::from_slice(payload).map_err(|_| UsageError::Invalid)?;
        let Some(limits) = value.get("rate_limits").filter(|value| !value.is_null()) else {
            return Ok(None);
        };
        let mut windows = Vec::new();
        for (key, minutes) in [("five_hour", 300), ("seven_day", 10080)] {
            if let Some(value) = limits.get(key).filter(|value| !value.is_null()) {
                windows.push(window(value, "used_percentage", minutes)?);
            }
        }
        if windows.is_empty() {
            return Ok(None);
        }
        let plan = std::fs::read(&self.claude_account)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
            .and_then(|value| {
                let account = value.get("oauthAccount")?;
                let organization = account
                    .get("organizationType")
                    .and_then(serde_json::Value::as_str);
                let tier = account
                    .get("organizationRateLimitTier")
                    .and_then(serde_json::Value::as_str);
                let parts: Vec<_> = [organization, tier]
                    .into_iter()
                    .flatten()
                    .filter(|value| !value.is_empty())
                    .collect();
                (!parts.is_empty()).then(|| parts.join(" · "))
            });
        Ok(Some(ProviderUsage {
            provider: ProviderKind::Claude,
            windows,
            plan,
        }))
    }

    async fn codex(&self, session: &str) -> Result<Option<(String, ProviderUsage)>, UsageError> {
        let home = self.codex_home.clone();
        let session = session.to_string();
        crate::common::operation_context::spawn_blocking(move || read_codex(&home, &session))
            .await
            .map_err(|error| UsageError::Technical(error.into()))?
    }
}

fn window(value: &serde_json::Value, field: &str, minutes: u64) -> Result<UsageWindow, UsageError> {
    let used_percent = value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .ok_or(UsageError::Invalid)?;
    let resets_at = value
        .get("resets_at")
        .and_then(serde_json::Value::as_u64)
        .ok_or(UsageError::Invalid)?;
    if !crate::domain::provider_usage::valid_window(used_percent, minutes, resets_at) {
        return Err(UsageError::Invalid);
    }
    Ok(UsageWindow {
        used_percent,
        window_minutes: minutes,
        resets_at,
        warning: crate::domain::provider_usage::warns(used_percent),
    })
}

pub(crate) fn read_codex(
    home: &std::path::Path,
    session: &str,
) -> Result<Option<(String, ProviderUsage)>, UsageError> {
    use rusqlite::OptionalExtension;
    let database = home.join("state_5.sqlite");
    if !database.exists() {
        return Ok(None);
    }
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|error| UsageError::Technical(error.into()))?;
    let path: Option<String> = connection
        .query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            [session],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| UsageError::Technical(error.into()))?;
    let Some(path) = path else { return Ok(None) };
    let mut lines =
        crate::infrastructure::reverse_lines::ReverseLines::open(std::path::Path::new(&path))
            .map_err(|error| UsageError::Technical(error.into()))?;
    while let Some(line) = lines
        .read_previous()
        .map_err(|error| UsageError::Technical(error.into()))?
    {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        #[derive(serde::Deserialize)]
        struct Header {
            #[serde(rename = "type")]
            kind: String,
        }
        #[derive(serde::Deserialize)]
        struct Event {
            payload: Payload,
        }
        #[derive(serde::Deserialize)]
        struct Payload {
            #[serde(rename = "type")]
            kind: String,
            rate_limits: Option<serde_json::Value>,
        }
        let header: Header = serde_json::from_slice(&line).map_err(|_| UsageError::Invalid)?;
        if header.kind != "event_msg" {
            continue;
        }
        let event: Event = serde_json::from_slice(&line).map_err(|_| UsageError::Invalid)?;
        if event.payload.kind != "token_count" {
            continue;
        }
        let Some(limits) = event.payload.rate_limits else {
            return Ok(None);
        };
        let mut windows = Vec::new();
        for key in ["primary", "secondary"] {
            if let Some(value) = limits.get(key).filter(|value| !value.is_null()) {
                let minutes = value
                    .get("window_minutes")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or(UsageError::Invalid)?;
                windows.push(window(value, "used_percent", minutes)?);
            }
        }
        if windows.is_empty() {
            return Ok(None);
        }
        return Ok(Some((
            {
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(&line))
            },
            ProviderUsage {
                provider: ProviderKind::Codex,
                windows,
                plan: limits
                    .get("plan_type")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            },
        )));
    }
    Ok(None)
}

#[cfg(test)]
#[path = "provider_usage_test.rs"]
mod provider_usage_tests;
