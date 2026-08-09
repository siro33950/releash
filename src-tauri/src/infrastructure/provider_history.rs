use std::fs;
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

fn normalize_timestamp_ms(value: i64) -> i64 {
    if value.unsigned_abs() < 10_000_000_000 {
        value.saturating_mul(1_000)
    } else {
        value
    }
}
