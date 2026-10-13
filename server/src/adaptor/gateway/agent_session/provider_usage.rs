use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::provider_usage::{
    CodexUsageObservation, CodexUsageRecord, ProviderUsage, ProviderUsageClock,
    ProviderUsageRepository, ProviderUsageSource, UsageError, UsageWindow,
};
use crate::usecase::provider_usage::{ProviderUsageDto, ProviderUsageQueryService, UsageWindowDto};
use std::path::PathBuf;
#[derive(Default)]
pub struct ProviderUsageReadModel {
    values: parking_lot::RwLock<Vec<ProviderUsage>>,
    observations: parking_lot::RwLock<std::collections::HashMap<String, CodexUsageObservation>>,
}
impl ProviderUsageQueryService for ProviderUsageReadModel {
    fn snapshot(&self) -> Vec<ProviderUsageDto> {
        self.values
            .read()
            .iter()
            .map(|usage| ProviderUsageDto {
                provider: usage.provider,
                windows: usage
                    .windows
                    .iter()
                    .map(|window| UsageWindowDto {
                        used_percent: window.used_percent(),
                        window_minutes: window.window_minutes(),
                        resets_at: window.resets_at(),
                        warning: window.warning(),
                    })
                    .collect(),
                plan: usage.plan.clone(),
            })
            .collect()
    }
}
impl ProviderUsageRepository for ProviderUsageReadModel {
    fn replace(&self, usage: ProviderUsage) {
        let mut values = self.values.write();
        if let Some(value) = values
            .iter_mut()
            .find(|value| value.provider == usage.provider)
        {
            *value = usage;
        } else {
            values.push(usage);
        }
    }
    fn start_observation(&self, observation: CodexUsageObservation) {
        self.observations
            .write()
            .insert(observation.session.clone(), observation);
    }
    fn has_observation(&self, session: &str) -> bool {
        self.observations.read().contains_key(session)
    }
    fn accept_record(&self, session: &str, record: &CodexUsageRecord) -> bool {
        self.observations
            .write()
            .get_mut(session)
            .is_some_and(|observation| observation.accept(record))
    }
    fn retain_observations(&self, sessions: &[String], poll_started_at: std::time::SystemTime) {
        self.observations
            .write()
            .retain(|_, observation| observation.retain_for_poll(sessions, poll_started_at));
    }
}
pub struct SystemProviderUsageClock;
impl ProviderUsageClock for SystemProviderUsageClock {
    fn now(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
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
        for (key, make_window) in [
            (
                "five_hour",
                UsageWindow::five_hour as fn(f64, u64) -> Result<UsageWindow, UsageError>,
            ),
            ("seven_day", UsageWindow::seven_day),
        ] {
            if let Some(value) = limits.get(key).filter(|value| !value.is_null()) {
                let (percent, reset) = window_values(value, "used_percentage")?;
                windows.push(make_window(percent, reset)?);
            }
        }
        if windows.is_empty() {
            return Ok(None);
        }
        let account = std::fs::read(&self.claude_account)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok());
        let account = account.as_ref().and_then(|value| value.get("oauthAccount"));
        let field = |key| {
            account
                .and_then(|account| account.get(key))
                .and_then(serde_json::Value::as_str)
        };
        Ok(Some(ProviderUsage::claude(
            windows,
            field("organizationType"),
            field("organizationRateLimitTier"),
        )))
    }

    async fn codex(&self, session: &str) -> Result<Option<CodexUsageRecord>, UsageError> {
        let home = self.codex_home.clone();
        let session = session.to_string();
        crate::common::operation_context::spawn_blocking(move || read_codex(&home, &session))
            .await
            .map_err(|error| {
                UsageError::from(crate::domain::failure::TechnicalFailure::from(error))
            })?
    }
}

fn window(value: &serde_json::Value, field: &str, minutes: u64) -> Result<UsageWindow, UsageError> {
    let (percent, reset) = window_values(value, field)?;
    UsageWindow::new(percent, minutes, reset)
}
fn window_values(value: &serde_json::Value, field: &str) -> Result<(f64, u64), UsageError> {
    let used_percent = value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .ok_or(UsageError::Invalid)?;
    let resets_at = value
        .get("resets_at")
        .and_then(serde_json::Value::as_u64)
        .ok_or(UsageError::Invalid)?;
    Ok((used_percent, resets_at))
}

pub(crate) fn read_codex(
    home: &std::path::Path,
    session: &str,
) -> Result<Option<CodexUsageRecord>, UsageError> {
    use rusqlite::OptionalExtension;
    let database = home.join("state_5.sqlite");
    if !database.exists() {
        return Ok(None);
    }
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|error| UsageError::from(crate::domain::failure::TechnicalFailure::from(error)))?;
    let path: Option<String> = connection
        .query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            [session],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| UsageError::from(crate::domain::failure::TechnicalFailure::from(error)))?;
    let Some(path) = path else { return Ok(None) };
    let mut lines =
        crate::infrastructure::reverse_lines::ReverseLines::open(std::path::Path::new(&path))
            .map_err(|error| {
                UsageError::from(crate::domain::failure::TechnicalFailure::from(error))
            })?;
    while let Some(line) = lines
        .read_previous()
        .map_err(|error| UsageError::from(crate::domain::failure::TechnicalFailure::from(error)))?
    {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        #[derive(serde::Deserialize)]
        struct Event {
            #[serde(rename = "type")]
            kind: String,
            timestamp: Option<String>,
            payload: Option<Payload>,
        }
        #[derive(serde::Deserialize)]
        struct Payload {
            #[serde(rename = "type")]
            kind: Option<String>,
            rate_limits: Option<serde_json::Value>,
        }
        let event: Event = serde_json::from_slice(&line).map_err(|_| UsageError::Invalid)?;
        if event.kind != "event_msg" {
            continue;
        }
        let Some(payload) = event.payload else {
            continue;
        };
        if payload.kind.as_deref() != Some("token_count") {
            continue;
        }
        let Some(limits) = payload.rate_limits else {
            return Ok(None);
        };
        let timestamp = parse_timestamp(
            &connection,
            event.timestamp.as_deref().ok_or(UsageError::Invalid)?,
        )?;
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
        return Ok(Some(CodexUsageRecord {
            id: {
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(&line))
            },
            timestamp,
            usage: ProviderUsage {
                provider: ProviderKind::Codex,
                windows,
                plan: limits
                    .get("plan_type")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            },
        }));
    }
    Ok(None)
}

fn parse_timestamp(
    connection: &rusqlite::Connection,
    value: &str,
) -> Result<std::time::SystemTime, UsageError> {
    let (whole, nanos) = if let Some((prefix, suffix)) = value.split_once('.') {
        let digits = suffix.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return Err(UsageError::Invalid);
        }
        let fraction = suffix[..digits]
            .parse::<u32>()
            .map_err(|_| UsageError::Invalid)?;
        (
            format!("{prefix}{}", &suffix[digits..]),
            fraction * 10u32.pow(9 - digits as u32),
        )
    } else {
        (value.to_string(), 0)
    };
    let bytes = whole.as_bytes();
    if bytes.len() < 20
        || bytes[..19]
            .iter()
            .enumerate()
            .any(|(index, byte)| match index {
                4 | 7 => *byte != b'-',
                10 => *byte != b'T',
                13 | 16 => *byte != b':',
                _ => !byte.is_ascii_digit(),
            })
        || !(whole.ends_with('Z')
            || bytes
                .get(19)
                .is_some_and(|byte| matches!(byte, b'+' | b'-')))
    {
        return Err(UsageError::Invalid);
    }
    let seconds: Option<i64> = connection
        .query_row("SELECT unixepoch(?1)", [&whole], |row| row.get(0))
        .map_err(|error| UsageError::from(crate::domain::failure::TechnicalFailure::from(error)))?;
    let seconds = seconds
        .filter(|seconds| *seconds >= 0)
        .ok_or(UsageError::Invalid)?;
    std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::new(seconds as u64, nanos))
        .ok_or(UsageError::Invalid)
}

#[cfg(test)]
#[path = "provider_usage_test.rs"]
mod provider_usage_tests;
