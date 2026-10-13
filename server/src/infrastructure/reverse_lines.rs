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
        loop {
            if let Some(index) = self.buffer.iter().rposition(|byte| *byte == b'\n') {
                let line = self.buffer.split_off(index + 1);
                self.buffer.pop();
                if self.first {
                    self.first = false;
                    continue;
                }
                return Ok(Some(line));
            }
            if self.position == 0 {
                if self.first || self.buffer.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(std::mem::take(&mut self.buffer)));
            }
            let start = self.position.saturating_sub(64 * 1024);
            let mut chunk = vec![0; (self.position - start) as usize];
            self.file.seek(SeekFrom::Start(start))?;
            self.file.read_exact(&mut chunk)?;
            chunk.append(&mut self.buffer);
            self.buffer = chunk;
            self.position = start;
        }
    }
}
