#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct NativeTerminalCheckpoint {
    pub(crate) replay: String,
    pub(crate) sequence: u64,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

pub(crate) struct NativeTerminalEmulator {
    terminal: avt::Vt,
}

impl NativeTerminalEmulator {
    pub(crate) fn new(cols: u16, rows: u16, scrollback_rows: usize) -> Self {
        let terminal = avt::Vt::builder()
            .size(usize::from(cols), usize::from(rows))
            .scrollback_limit(scrollback_rows)
            .build();
        Self { terminal }
    }

    pub(crate) fn restore(checkpoint: &NativeTerminalCheckpoint, scrollback_rows: usize) -> Self {
        let mut terminal = avt::Vt::builder()
            .size(usize::from(checkpoint.cols), usize::from(checkpoint.rows))
            .scrollback_limit(scrollback_rows)
            .build();
        terminal.feed_str(&checkpoint.replay);
        Self { terminal }
    }

    pub(crate) fn apply(&mut self, output: &str) {
        self.terminal.feed_str(output);
    }

    pub(crate) fn resize(&mut self, cols: u16, rows: u16) {
        self.terminal.resize(usize::from(cols), usize::from(rows));
    }

    pub(crate) fn snapshot(&self, sequence: u64) -> NativeTerminalCheckpoint {
        let (cols, rows) = self.terminal.size();
        NativeTerminalCheckpoint {
            replay: self.replay(),
            sequence,
            cols: u16::try_from(cols).expect("Terminal Surface columns originated as u16"),
            rows: u16::try_from(rows).expect("Terminal Surface rows originated as u16"),
        }
    }

    fn replay(&self) -> String {
        let (cols, rows) = self.terminal.size();
        let dump = self.terminal.dump();
        let mut replay = String::from("\x1bc");
        let physical_history_len = self.terminal.lines().count().saturating_sub(rows);
        if physical_history_len > 0 {
            for line in self.terminal.lines().take(physical_history_len) {
                append_line_ansi(&mut replay, line);
                replay.push_str("\r\n");
            }
        } else {
            let mut visible = avt::Vt::builder()
                .size(cols, rows)
                .scrollback_limit(0)
                .build();
            visible.feed_str(&dump);
            let primary = self.terminal.text();
            let visible_primary = visible.text();
            let logical_history_len = primary
                .len()
                .checked_sub(visible_primary.len())
                .filter(|&start| primary[start..] == visible_primary)
                .unwrap_or(0);
            for line in &primary[..logical_history_len] {
                replay.push_str(line);
                replay.push_str("\r\n");
            }
        }
        for _ in 0..rows {
            replay.push_str("\r\n");
        }
        replay.push_str("\x1b[H\x1b[2J");
        replay.push_str(&dump);
        replay
    }
}

fn append_line_ansi(output: &mut String, line: &avt::Line) {
    let cells = line.cells();
    let end = cells
        .iter()
        .rposition(|cell| !cell.is_default())
        .map_or(0, |index| index + 1);
    let mut pen = avt::Pen::default();
    for cell in &cells[..end] {
        if cell.width() == 0 {
            continue;
        }
        if cell.pen() != &pen {
            pen = *cell.pen();
            append_pen_ansi(output, &pen);
        }
        output.push(cell.char());
    }
    if !pen.is_default() {
        output.push_str("\x1b[0m");
    }
}

fn append_pen_ansi(output: &mut String, pen: &avt::Pen) {
    let mut codes = vec!["0".to_string()];
    if pen.is_bold() {
        codes.push("1".to_string());
    }
    if pen.is_faint() {
        codes.push("2".to_string());
    }
    if pen.is_italic() {
        codes.push("3".to_string());
    }
    if pen.is_underline() {
        codes.push("4".to_string());
    }
    if pen.is_blink() {
        codes.push("5".to_string());
    }
    if pen.is_inverse() {
        codes.push("7".to_string());
    }
    if pen.is_strikethrough() {
        codes.push("9".to_string());
    }
    append_color_codes(&mut codes, pen.foreground(), true);
    append_color_codes(&mut codes, pen.background(), false);
    output.push_str("\x1b[");
    output.push_str(&codes.join(";"));
    output.push('m');
}

fn append_color_codes(codes: &mut Vec<String>, color: Option<avt::Color>, foreground: bool) {
    let Some(color) = color else {
        return;
    };
    match color {
        avt::Color::Indexed(index) if index < 8 => {
            codes.push((if foreground { 30 } else { 40 } + u16::from(index)).to_string());
        }
        avt::Color::Indexed(index) if index < 16 => {
            codes.push((if foreground { 90 } else { 100 } + u16::from(index - 8)).to_string());
        }
        avt::Color::Indexed(index) => {
            codes.push(if foreground { "38" } else { "48" }.to_string());
            codes.push("5".to_string());
            codes.push(index.to_string());
        }
        avt::Color::RGB(rgb) => {
            codes.push(if foreground { "38" } else { "48" }.to_string());
            codes.push("2".to_string());
            codes.push(rgb.r.to_string());
            codes.push(rgb.g.to_string());
            codes.push(rgb.b.to_string());
        }
    }
}

#[derive(Clone)]
pub(crate) struct TerminalCheckpointFileStore {
    root: std::path::PathBuf,
    scrollback_rows: usize,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct StoredTerminalCheckpointBase {
    version: u8,
    session_key: String,
    checkpoint: NativeTerminalCheckpoint,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum NativeTerminalCheckpointRecord {
    Output {
        sequence: u64,
        data: std::sync::Arc<str>,
    },
    Resize {
        sequence: u64,
        cols: u16,
        rows: u16,
    },
    Barrier {
        sequence: u64,
    },
}

impl NativeTerminalCheckpointRecord {
    pub(crate) fn sequence(&self) -> u64 {
        match self {
            Self::Output { sequence, .. }
            | Self::Resize { sequence, .. }
            | Self::Barrier { sequence } => *sequence,
        }
    }
}

impl TerminalCheckpointFileStore {
    pub(crate) fn new(app_data_dir: &std::path::Path, scrollback_rows: usize) -> Self {
        Self {
            root: app_data_dir.join("terminal-surfaces"),
            scrollback_rows,
        }
    }

    pub(crate) fn load(
        &self,
        session_key: &str,
    ) -> Result<Option<NativeTerminalCheckpoint>, String> {
        let path = self.path_for(session_key);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("read {}: {error}", path.display())),
        };
        let stored: StoredTerminalCheckpointBase = serde_json::from_slice(&bytes)
            .map_err(|error| format!("decode {}: {error}", path.display()))?;
        if stored.version != 2 || stored.session_key != session_key {
            return Err(format!(
                "invalid Terminal Surface checkpoint: {}",
                path.display()
            ));
        }
        let mut sequence = stored.checkpoint.sequence;
        let mut terminal =
            NativeTerminalEmulator::restore(&stored.checkpoint, self.scrollback_rows);
        let journal_path = self.journal_path_for(session_key);
        let journal = match std::fs::read(&journal_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some(stored.checkpoint));
            }
            Err(error) => return Err(format!("read {}: {error}", journal_path.display())),
        };
        let durable_len = journal
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        if durable_len < journal.len() {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&journal_path)
                .map_err(|error| format!("repair {}: {error}", journal_path.display()))?;
            file.set_len(durable_len as u64)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("repair {}: {error}", journal_path.display()))?;
        }
        for line in journal[..durable_len].split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            let record: NativeTerminalCheckpointRecord = serde_json::from_slice(line)
                .map_err(|error| format!("decode {}: {error}", journal_path.display()))?;
            if record.sequence() <= sequence {
                continue;
            }
            if record.sequence() != sequence + 1 {
                return Err(format!(
                    "non-contiguous Terminal Surface journal: {}",
                    journal_path.display()
                ));
            }
            match &record {
                NativeTerminalCheckpointRecord::Output { data, .. } => terminal.apply(data),
                NativeTerminalCheckpointRecord::Resize { cols, rows, .. } => {
                    terminal.resize(*cols, *rows);
                }
                NativeTerminalCheckpointRecord::Barrier { .. } => {}
            }
            sequence = record.sequence();
        }
        Ok(Some(terminal.snapshot(sequence)))
    }

    #[cfg(test)]
    pub(crate) fn save(
        &self,
        session_key: &str,
        checkpoint: &NativeTerminalCheckpoint,
    ) -> Result<(), String> {
        self.replace_base(session_key, checkpoint)
    }

    pub(crate) fn replace_base(
        &self,
        session_key: &str,
        checkpoint: &NativeTerminalCheckpoint,
    ) -> Result<(), String> {
        self.create_private_root()?;
        let path = self.path_for(session_key);
        let temp = self.root.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec(&StoredTerminalCheckpointBase {
            version: 2,
            session_key: session_key.to_string(),
            checkpoint: checkpoint.clone(),
        })
        .map_err(|error| format!("encode Terminal Surface checkpoint: {error}"))?;
        let write_result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            #[cfg(unix)]
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
            std::fs::rename(&temp, &path)
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("write {}: {error}", path.display()));
        }
        let journal_path = self.journal_path_for(session_key);
        match std::fs::remove_file(&journal_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("delete {}: {error}", journal_path.display())),
        }
        Ok(())
    }

    pub(crate) fn append_records(
        &self,
        session_key: &str,
        records: &[NativeTerminalCheckpointRecord],
    ) -> Result<usize, String> {
        if records.is_empty() {
            return Ok(0);
        }
        self.create_private_root()?;
        let path = self.journal_path_for(session_key);
        let mut bytes = Vec::new();
        for record in records {
            serde_json::to_writer(&mut bytes, record)
                .map_err(|error| format!("encode Terminal Surface journal: {error}"))?;
            bytes.push(b'\n');
        }
        let write_result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&path)?;
            #[cfg(unix)]
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        write_result.map_err(|error| format!("write {}: {error}", path.display()))?;
        Ok(bytes.len())
    }

    pub(crate) fn journal_len(&self, session_key: &str) -> Result<u64, String> {
        match std::fs::metadata(self.journal_path_for(session_key)) {
            Ok(metadata) => Ok(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(crate) fn delete(&self, session_key: &str) -> Result<(), String> {
        let path = self.path_for(session_key);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("delete {}: {error}", path.display())),
        }
        let journal = self.journal_path_for(session_key);
        match std::fs::remove_file(&journal) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("delete {}: {error}", journal.display())),
        }
    }

    fn create_private_root(&self) -> Result<(), String> {
        let result = (|| -> std::io::Result<()> {
            #[cfg(unix)]
            {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&self.root)?;
                std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(&self.root)
            }
        })();
        result.map_err(|error| format!("create {}: {error}", self.root.display()))
    }

    fn path_for(&self, session_key: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(session_key.as_bytes());
        self.root
            .join(format!("{}.base-v2.json", hex::encode(digest)))
    }

    fn journal_path_for(&self, session_key: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(session_key.as_bytes());
        self.root
            .join(format!("{}.journal-v2.jsonl", hex::encode(digest)))
    }
}

#[cfg(test)]
#[path = "terminal_emulator_test.rs"]
mod terminal_emulator_tests;
