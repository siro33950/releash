use std::path::{Path, PathBuf};

use super::layout::{
    attachments_dir_in_dir, messages_dir_in_dir, meta_file_in_dir, sessions_dir,
    tool_outputs_dir_in_dir,
};
use super::FileSessionStorage;
use crate::usecase::agent_session::session::SessionState;

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionGcMeta {
    pub(crate) id: Option<String>,
    pub(crate) worktree_path: Option<String>,
    pub(crate) state: Option<SessionState>,
    pub(crate) updated_at: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionGcRecordLayout {
    pub(crate) id: String,
    pub(crate) delete_paths: Vec<PathBuf>,
    pub(crate) dir_path: Option<PathBuf>,
    pub(crate) worktree_path: Option<String>,
    pub(crate) state: Option<SessionState>,
    pub(crate) updated_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionGcBlobStoreLayout {
    pub(crate) session_dir: PathBuf,
    pub(crate) messages_dir: PathBuf,
    pub(crate) tool_outputs_dir: PathBuf,
    pub(crate) attachments_dir: PathBuf,
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

#[derive(Debug, Clone)]
pub(crate) enum SessionGcMetaRead {
    Present(SessionGcMeta),
    Missing,
    Unavailable(String),
}

#[derive(Clone, Copy)]
enum InvalidJsonPolicy {
    DefaultMeta,
    Error,
}

enum SessionGcMetaReadError {
    NotFound,
    Unavailable(String),
}

impl FileSessionStorage {
    pub(crate) fn list_gc_session_records(
        &self,
        app_data_dir: &Path,
    ) -> Vec<SessionGcRecordLayout> {
        let sessions_dir = sessions_dir(app_data_dir);
        let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let Some(id) = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                let meta = match read_gc_meta_or_default_on_missing(
                    &meta_file_in_dir(&path),
                    &id,
                    InvalidJsonPolicy::DefaultMeta,
                ) {
                    Some(meta) => meta,
                    None => continue,
                };
                records.push(session_record_layout(
                    meta,
                    &id,
                    vec![path.clone()],
                    Some(path),
                ));
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if stem.ends_with(".meta") {
                continue;
            }
            let id = stem.to_string();
            let sidecar = legacy_meta_file_in_sessions_dir(&sessions_dir, &id);
            let meta_path = if sidecar.exists() { &sidecar } else { &path };
            let meta = match read_gc_meta_or_default_on_missing(
                meta_path,
                &id,
                InvalidJsonPolicy::DefaultMeta,
            ) {
                Some(meta) => meta,
                None => continue,
            };
            let mut delete_paths = vec![path];
            if sidecar.exists() {
                delete_paths.push(sidecar);
            }
            records.push(session_record_layout(meta, &id, delete_paths, None));
        }
        records
    }

    pub(crate) fn list_gc_session_blob_stores(
        &self,
        app_data_dir: &Path,
    ) -> Vec<SessionGcBlobStoreLayout> {
        let sessions_dir = sessions_dir(app_data_dir);
        let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .map(|session_dir| SessionGcBlobStoreLayout {
                messages_dir: messages_dir_in_dir(&session_dir),
                tool_outputs_dir: tool_outputs_dir_in_dir(&session_dir),
                attachments_dir: attachments_dir_in_dir(&session_dir),
                session_dir,
            })
            .collect()
    }

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
            match read_session_gc_meta_file(&meta_path, &fallback_id, InvalidJsonPolicy::Error) {
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

    pub(crate) fn read_gc_session_meta_for_revalidation(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> SessionGcMetaRead {
        let sessions_dir = sessions_dir(app_data_dir);
        let session_dir = sessions_dir.join(session_id);
        if session_dir.is_dir() {
            return read_gc_meta_for_revalidation(&meta_file_in_dir(&session_dir), session_id);
        }
        let session_file = sessions_dir.join(format!("{session_id}.json"));
        let sidecar = legacy_meta_file_in_sessions_dir(&sessions_dir, session_id);
        if sidecar.exists() {
            return read_gc_meta_for_revalidation(&sidecar, session_id);
        }
        if session_file.exists() {
            return read_gc_meta_for_revalidation(&session_file, session_id);
        }
        SessionGcMetaRead::Missing
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

fn session_record_layout(
    meta: SessionGcMeta,
    fallback_id: &str,
    delete_paths: Vec<PathBuf>,
    dir_path: Option<PathBuf>,
) -> SessionGcRecordLayout {
    SessionGcRecordLayout {
        id: meta.id.unwrap_or_else(|| fallback_id.to_string()),
        delete_paths,
        dir_path,
        worktree_path: meta.worktree_path,
        state: meta.state,
        updated_at: meta.updated_at,
    }
}

fn read_gc_meta_for_revalidation(path: &Path, fallback_id: &str) -> SessionGcMetaRead {
    match read_session_gc_meta_file(path, fallback_id, InvalidJsonPolicy::DefaultMeta) {
        Ok(meta) => SessionGcMetaRead::Present(meta),
        Err(SessionGcMetaReadError::NotFound) => SessionGcMetaRead::Present(SessionGcMeta {
            id: Some(fallback_id.to_string()),
            ..SessionGcMeta::default()
        }),
        Err(SessionGcMetaReadError::Unavailable(error)) => SessionGcMetaRead::Unavailable(error),
    }
}

fn read_gc_meta_or_default_on_missing(
    path: &Path,
    fallback_id: &str,
    invalid_json: InvalidJsonPolicy,
) -> Option<SessionGcMeta> {
    match read_session_gc_meta_file(path, fallback_id, invalid_json) {
        Ok(meta) => Some(meta),
        Err(SessionGcMetaReadError::NotFound) => Some(SessionGcMeta {
            id: Some(fallback_id.to_string()),
            ..SessionGcMeta::default()
        }),
        Err(SessionGcMetaReadError::Unavailable(error)) => {
            log::warn!(
                "app data gc skipped session {} because meta {} could not be read: {error}",
                fallback_id,
                path.display()
            );
            None
        }
    }
}

fn read_session_gc_meta_file(
    path: &Path,
    fallback_id: &str,
    invalid_json: InvalidJsonPolicy,
) -> Result<SessionGcMeta, SessionGcMetaReadError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(SessionGcMetaReadError::NotFound);
        }
        Err(error) => return Err(SessionGcMetaReadError::Unavailable(error.to_string())),
    };
    let value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => value,
        Err(error) => match invalid_json {
            InvalidJsonPolicy::DefaultMeta => {
                log::warn!(
                    "app data gc skipped unreadable session meta {}",
                    path.display()
                );
                return Ok(SessionGcMeta {
                    id: Some(fallback_id.to_string()),
                    ..SessionGcMeta::default()
                });
            }
            InvalidJsonPolicy::Error => {
                return Err(SessionGcMetaReadError::Unavailable(error.to_string()));
            }
        },
    };
    if matches!(invalid_json, InvalidJsonPolicy::Error) {
        validate_session_gc_meta_shape(&value).map_err(SessionGcMetaReadError::Unavailable)?;
    }
    Ok(session_gc_meta_from_value(&value, fallback_id))
}

fn validate_session_gc_meta_shape(value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("session meta must be a JSON object".to_string());
    }
    validate_optional_string_field(value, "id")?;
    validate_optional_string_field(value, "worktreePath")?;
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
        updated_at: value.get("updatedAt").and_then(|value| value.as_f64()),
    }
}

fn legacy_meta_file_in_sessions_dir(sessions_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir.join(format!("{session_id}.meta.json"))
}
