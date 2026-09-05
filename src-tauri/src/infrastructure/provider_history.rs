use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawHistoryFile {
    pub(crate) path: PathBuf,
    pub(crate) updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawCodexHistoryRow {
    pub(crate) session_id: String,
    pub(crate) cwd: String,
    pub(crate) updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawCodexThreadName {
    pub(crate) session_id: String,
    pub(crate) name: Option<String>,
    pub(crate) first_user_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawFileTail {
    pub(crate) preceding_byte: Option<u8>,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawFileHead {
    pub(crate) bytes: Vec<u8>,
    pub(crate) following_byte: Option<u8>,
}

pub(crate) fn read_file_head(path: &Path, max_bytes: usize) -> std::io::Result<RawFileHead> {
    if max_bytes == 0 {
        return Ok(RawFileHead {
            bytes: Vec::new(),
            following_byte: None,
        });
    }
    let mut file = File::open(path)?;
    let max_bytes = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let mut bytes = Vec::with_capacity(usize::try_from(max_bytes).unwrap_or(usize::MAX));
    file.by_ref().take(max_bytes).read_to_end(&mut bytes)?;
    let mut following = [0];
    let following_byte = (file.read(&mut following)? == 1).then_some(following[0]);
    Ok(RawFileHead {
        bytes,
        following_byte,
    })
}

pub(crate) fn read_file_tail(path: &Path, max_bytes: usize) -> std::io::Result<RawFileTail> {
    if max_bytes == 0 {
        return Ok(RawFileTail {
            preceding_byte: None,
            bytes: Vec::new(),
        });
    }
    let mut file = File::open(path)?;
    let end = file.seek(SeekFrom::End(0))?;
    let max_bytes = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    let start = end.saturating_sub(max_bytes);
    let preceding_byte = if start == 0 {
        None
    } else {
        file.seek(SeekFrom::Start(start - 1))?;
        let mut byte = [0];
        file.read_exact(&mut byte)?;
        Some(byte[0])
    };
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(usize::try_from(end.min(max_bytes)).unwrap_or(usize::MAX));
    file.take(max_bytes).read_to_end(&mut bytes)?;
    Ok(RawFileTail {
        preceding_byte,
        bytes,
    })
}

pub(crate) fn recent_jsonl_files(
    directory: &Path,
    limit: usize,
) -> std::io::Result<Vec<RawHistoryFile>> {
    if limit == 0 || !directory.exists() {
        return Ok(Vec::new());
    }
    let mut recent = Vec::with_capacity(limit);
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
        {
            continue;
        }
        let modified = entry.metadata()?.modified().unwrap_or(UNIX_EPOCH);
        let updated_at_ms = modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or(0);
        recent.push(RawHistoryFile {
            path,
            updated_at_ms,
        });
        recent.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.path.cmp(&right.path))
        });
        recent.truncate(limit);
    }
    Ok(recent)
}

pub(crate) fn query_codex_history(
    database: &Path,
    worktree_path: &str,
    limit: usize,
) -> Result<Vec<RawCodexHistoryRow>, rusqlite::Error> {
    if limit == 0 || !database.exists() {
        return Ok(Vec::new());
    }
    let connection = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut statement = connection.prepare(
        "SELECT id, cwd, updated_at FROM threads \
         WHERE cwd = ?1 AND id IS NOT NULL AND id != '' \
         ORDER BY updated_at DESC, id ASC LIMIT ?2",
    )?;
    let rows = statement.query_map(
        rusqlite::params![worktree_path, i64::try_from(limit).unwrap_or(i64::MAX)],
        |row| {
            let updated_at: i64 = row.get(2)?;
            Ok(RawCodexHistoryRow {
                session_id: row.get(0)?,
                cwd: row.get(1)?,
                updated_at_ms: normalize_timestamp_ms(updated_at),
            })
        },
    )?;
    rows.collect()
}

pub(crate) fn query_codex_thread_names(
    database: &Path,
    provider_session_ids: &[String],
) -> Result<Vec<RawCodexThreadName>, rusqlite::Error> {
    if provider_session_ids.is_empty() || !database.exists() {
        return Ok(Vec::new());
    }
    let connection = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let placeholders = (1..=provider_session_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = connection.prepare(&format!(
        "SELECT id, name, first_user_message FROM threads \
         WHERE id IN ({placeholders}) ORDER BY id"
    ))?;
    let rows = statement.query_map(
        rusqlite::params_from_iter(provider_session_ids.iter()),
        |row| {
            Ok(RawCodexThreadName {
                session_id: row.get(0)?,
                name: row.get(1)?,
                first_user_message: row.get(2)?,
            })
        },
    )?;
    rows.collect()
}

fn normalize_timestamp_ms(value: i64) -> i64 {
    if value.unsigned_abs() < 10_000_000_000 {
        value.saturating_mul(1_000)
    } else {
        value
    }
}
