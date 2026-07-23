use std::path::{Path, PathBuf};

use super::layout::{meta_file_in_dir, sessions_dir};
use super::FileSessionStorage;
use crate::usecase::agent_session::session::SessionState;

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionGcMeta {
    pub(crate) id: Option<String>,
    pub(crate) worktree_path: Option<String>,
    pub(crate) state: Option<SessionState>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionGcMetaScan {
    pub(crate) items: Vec<SessionGcMeta>,
    pub(crate) is_complete: bool,
}

impl Default for SessionGcMetaScan {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            is_complete: true,
        }
    }
}

enum SessionGcMetaReadError {
    NotFound,
    Unavailable(String),
}

impl FileSessionStorage {
    pub(crate) fn list_gc_session_protection_meta(&self, app_data_dir: &Path) -> SessionGcMetaScan {
        let sessions_dir = sessions_dir(app_data_dir);
        let entries = match std::fs::read_dir(&sessions_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return SessionGcMetaScan {
                    items: Vec::new(),
                    is_complete: true,
                };
            }
            Err(error) => {
                log::warn!(
                    "app data gc could not scan session protection metadata {}: {error}",
                    sessions_dir.display()
                );
                return SessionGcMetaScan {
                    items: Vec::new(),
                    is_complete: false,
                };
            }
        };
        let mut items = Vec::new();
        let mut is_complete = true;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    log::warn!(
                        "app data gc skipped session protection entry in {}: {error}",
                        sessions_dir.display()
                    );
                    is_complete = false;
                    continue;
                }
            };
            let path = entry.path();
            let Some((meta_path, fallback_id)) =
                gc_meta_path_for_session_entry(&sessions_dir, path)
            else {
                continue;
            };
            match read_session_gc_meta_file(&meta_path, &fallback_id) {
                Ok(meta) => items.push(meta),
                Err(SessionGcMetaReadError::NotFound) => {
                    log::warn!(
                        "app data gc could not read session protection metadata {}: not found",
                        meta_path.display()
                    );
                    is_complete = false;
                }
                Err(SessionGcMetaReadError::Unavailable(error)) => {
                    log::warn!(
                        "app data gc could not read session protection metadata {}: {error}",
                        meta_path.display()
                    );
                    is_complete = false;
                }
            }
        }
        SessionGcMetaScan { items, is_complete }
    }
}

fn gc_meta_path_for_session_entry(sessions_dir: &Path, path: PathBuf) -> Option<(PathBuf, String)> {
    if path.is_dir() {
        let id = path.file_name().and_then(|name| name.to_str())?.to_string();
        return Some((meta_file_in_dir(&path), id));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
        return None;
    }
    let stem = path.file_stem().and_then(|stem| stem.to_str())?.to_string();
    if stem.ends_with(".meta") {
        return None;
    }
    let sidecar = legacy_meta_file_in_sessions_dir(sessions_dir, &stem);
    if sidecar.exists() {
        Some((sidecar, stem))
    } else {
        Some((path, stem))
    }
}

fn read_session_gc_meta_file(
    path: &Path,
    fallback_id: &str,
) -> Result<SessionGcMeta, SessionGcMetaReadError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(SessionGcMetaReadError::NotFound);
        }
        Err(error) => return Err(SessionGcMetaReadError::Unavailable(error.to_string())),
    };
    let value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|error| SessionGcMetaReadError::Unavailable(error.to_string()))?;
    validate_session_gc_meta_shape(&value).map_err(SessionGcMetaReadError::Unavailable)?;
    Ok(session_gc_meta_from_value(&value, fallback_id))
}

fn validate_session_gc_meta_shape(value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("session meta must be a JSON object".to_string());
    }
    validate_optional_string_field(value, "id")?;
    validate_optional_string_field(value, "worktreePath")?;
    validate_optional_session_state_field(value, "state")?;
    validate_optional_number_field(value, "updatedAt")?;
    Ok(())
}

fn validate_optional_string_field(value: &serde_json::Value, field: &str) -> Result<(), String> {
    if value
        .get(field)
        .is_some_and(|value| !value.is_string() && !value.is_null())
    {
        return Err(format!("session meta field {field} must be a string"));
    }
    Ok(())
}

fn validate_optional_session_state_field(
    value: &serde_json::Value,
    field: &str,
) -> Result<(), String> {
    let Some(field_value) = value.get(field) else {
        return Ok(());
    };
    if field_value.is_null() {
        return Ok(());
    }
    serde_json::from_value::<SessionState>(field_value.clone())
        .map_err(|_| format!("session meta field {field} must be a valid session state"))?;
    Ok(())
}

fn validate_optional_number_field(value: &serde_json::Value, field: &str) -> Result<(), String> {
    if value
        .get(field)
        .is_some_and(|value| !value.is_number() && !value.is_null())
    {
        return Err(format!("session meta field {field} must be a number"));
    }
    Ok(())
}

fn session_gc_meta_from_value(value: &serde_json::Value, fallback_id: &str) -> SessionGcMeta {
    SessionGcMeta {
        id: value
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| Some(fallback_id.to_string())),
        worktree_path: value
            .get("worktreePath")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        state: value
            .get("state")
            .and_then(|value| serde_json::from_value::<SessionState>(value.clone()).ok()),
    }
}

fn legacy_meta_file_in_sessions_dir(sessions_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir.join(format!("{session_id}.meta.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_meta_validation_rejects_invalid_state_and_updated_at_types() {
        assert!(validate_session_gc_meta_shape(&serde_json::json!({
            "state": "not_a_state"
        }))
        .is_err());
        assert!(validate_session_gc_meta_shape(&serde_json::json!({
            "state": true
        }))
        .is_err());
        assert!(validate_session_gc_meta_shape(&serde_json::json!({
            "updatedAt": "100"
        }))
        .is_err());
    }

    #[test]
    fn strict_meta_validation_accepts_valid_state_updated_at_and_nulls() {
        validate_session_gc_meta_shape(&serde_json::json!({
            "state": "active",
            "updatedAt": 100.5
        }))
        .unwrap();
        validate_session_gc_meta_shape(&serde_json::json!({
            "state": null,
            "updatedAt": null
        }))
        .unwrap();
    }

    #[test]
    fn protection_meta_scan_marks_invalid_meta_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(sessions_dir.join("invalid-state")).unwrap();
        std::fs::write(
            sessions_dir.join("invalid-state/meta.json"),
            serde_json::json!({
                "id": "invalid-state",
                "state": "not_a_state"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(sessions_dir.join("invalid-updated-at")).unwrap();
        std::fs::write(
            sessions_dir.join("invalid-updated-at/meta.json"),
            serde_json::json!({
                "id": "invalid-updated-at",
                "updatedAt": "100"
            })
            .to_string(),
        )
        .unwrap();

        let scan = FileSessionStorage::default().list_gc_session_protection_meta(tmp.path());

        assert!(!scan.is_complete);
        assert!(scan.items.is_empty());
    }
}
