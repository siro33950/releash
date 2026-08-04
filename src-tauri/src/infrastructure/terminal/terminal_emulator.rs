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
}

#[derive(serde::Deserialize, serde::Serialize)]
struct StoredTerminalCheckpoint {
    version: u8,
    session_key: String,
    checkpoint: NativeTerminalCheckpoint,
}

impl TerminalCheckpointFileStore {
    pub(crate) fn new(app_data_dir: &std::path::Path) -> Self {
        Self {
            root: app_data_dir.join("terminal-surfaces"),
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
        let stored: StoredTerminalCheckpoint = serde_json::from_slice(&bytes)
            .map_err(|error| format!("decode {}: {error}", path.display()))?;
        if stored.version != 1 || stored.session_key != session_key {
            return Err(format!(
                "invalid Terminal Surface checkpoint: {}",
                path.display()
            ));
        }
        Ok(Some(stored.checkpoint))
    }

    pub(crate) fn save(
        &self,
        session_key: &str,
        checkpoint: &NativeTerminalCheckpoint,
    ) -> Result<(), String> {
        std::fs::create_dir_all(&self.root)
            .map_err(|error| format!("create {}: {error}", self.root.display()))?;
        let path = self.path_for(session_key);
        let temp = self.root.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec(&StoredTerminalCheckpoint {
            version: 1,
            session_key: session_key.to_string(),
            checkpoint: checkpoint.clone(),
        })
        .map_err(|error| format!("encode Terminal Surface checkpoint: {error}"))?;
        let write_result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temp, &path)
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("write {}: {error}", path.display()));
        }
        Ok(())
    }

    pub(crate) fn delete(&self, session_key: &str) -> Result<(), String> {
        let path = self.path_for(session_key);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("delete {}: {error}", path.display())),
        }
    }

    fn path_for(&self, session_key: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(session_key.as_bytes());
        self.root.join(format!("{}.json", hex::encode(digest)))
    }
}

#[cfg(test)]
#[path = "terminal_emulator_test.rs"]
mod terminal_emulator_tests;
