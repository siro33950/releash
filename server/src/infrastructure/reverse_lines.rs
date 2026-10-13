use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub struct ReverseLines {
    file: File,
    position: u64,
    buffer: Vec<u8>,
    first: bool,
}
impl ReverseLines {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let position = file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            position,
            buffer: Vec::new(),
            first: true,
        })
    }
    pub fn read_previous(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        let mut parts = Vec::new();
        loop {
            if let Some(index) = self.buffer.iter().rposition(|byte| *byte == b'\n') {
                parts.push(self.buffer.split_off(index + 1));
                self.buffer.pop();
                if self.first {
                    self.first = false;
                    parts.clear();
                    continue;
                }
                return Ok(Some(join(parts)));
            }
            if !self.buffer.is_empty() {
                parts.push(std::mem::take(&mut self.buffer));
            }
            if self.position == 0 {
                if self.first || parts.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(join(parts)));
            }
            let start = self.position.saturating_sub(64 * 1024);
            self.buffer = vec![0; (self.position - start) as usize];
            self.file.seek(SeekFrom::Start(start))?;
            self.file.read_exact(&mut self.buffer)?;
            self.position = start;
        }
    }
}
fn join(parts: Vec<Vec<u8>>) -> Vec<u8> {
    let mut line = Vec::with_capacity(parts.iter().map(Vec::len).sum());
    for part in parts.into_iter().rev() {
        line.extend(part);
    }
    line
}
