use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use super::wire_record::WireRecorder;

/// Shared stdout line limit for agent backends. This preserves Claude's existing 8 MB limit.
pub(crate) const MAX_STDOUT_LINE_BYTES: usize = 8 * 1024 * 1024;

const LINE_PROBE_BYTES: usize = 4 * 1024;
const MAX_KIND_HINT_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineProbe {
    pub kind_hint: Option<String>,
    pub bytes: usize,
}

#[derive(Debug, Default)]
pub(crate) struct StdoutDiagnostics {
    oversize_dropped_count: u64,
    skipped_non_json_count: u64,
}

impl StdoutDiagnostics {
    pub(crate) fn reset(&mut self) {
        self.oversize_dropped_count = 0;
        self.skipped_non_json_count = 0;
    }

    pub(crate) fn record_non_json_skip(&mut self, backend: &str, probe: &LineProbe) {
        self.skipped_non_json_count = self.skipped_non_json_count.saturating_add(1);
        log::warn!(
            "{backend} stdout skipped non-json line: bytes={} count={}",
            probe.bytes,
            self.skipped_non_json_count
        );
    }

    pub(crate) fn record_oversize_drop(&mut self, backend: &str, probe: &LineProbe) -> String {
        self.oversize_dropped_count = self.oversize_dropped_count.saturating_add(1);
        log::warn!(
            "{backend} stdout dropped oversized line: bytes={} kind_hint={} count={}",
            probe.bytes,
            probe.kind_hint.as_deref().unwrap_or("unknown"),
            self.oversize_dropped_count
        );
        let kind_hint = probe
            .kind_hint
            .as_deref()
            .map(|kind| format!("（推定種別: {kind}）"))
            .unwrap_or_default();
        format!(
            "backend からの応答 1 件がサイズ上限（{}MB）を超えたため破棄しました{kind_hint}",
            MAX_STDOUT_LINE_BYTES / (1024 * 1024)
        )
    }

    pub(crate) fn oversize_dropped_count(&self) -> u64 {
        self.oversize_dropped_count
    }

    #[cfg(test)]
    pub(crate) fn skipped_non_json_count(&self) -> u64 {
        self.skipped_non_json_count
    }
}

#[derive(Debug)]
pub(crate) enum StdoutItem {
    Json(Value),
    NonJson { probe: LineProbe },
    Oversize { probe: LineProbe },
}

pub(crate) struct StdoutLineReader<R> {
    inner: R,
    max_line_bytes: usize,
    wire_recorder: Option<WireRecorder>,
}

impl<R: AsyncBufRead + Unpin> StdoutLineReader<R> {
    #[cfg(test)]
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            max_line_bytes: MAX_STDOUT_LINE_BYTES,
            wire_recorder: None,
        }
    }

    pub(crate) fn with_wire_recorder(inner: R, wire_recorder: WireRecorder) -> Self {
        Self {
            inner,
            max_line_bytes: MAX_STDOUT_LINE_BYTES,
            wire_recorder: wire_recorder.is_active().then_some(wire_recorder),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_max_line_bytes(inner: R, max_line_bytes: usize) -> Self {
        Self {
            inner,
            max_line_bytes,
            wire_recorder: None,
        }
    }

    pub(crate) async fn shutdown_wire_recorder(&mut self) {
        if let Some(wire_recorder) = self.wire_recorder.as_mut() {
            wire_recorder.shutdown().await;
        }
    }

    /// Reads and classifies one logical line. Only I/O failures are returned as errors.
    pub(crate) async fn next(&mut self) -> Result<Option<StdoutItem>, String> {
        let mut line = Vec::new();
        let mut bytes = 0usize;
        let mut oversize = false;
        let mut oversize_kind_hint = None;

        loop {
            let available = self
                .inner
                .fill_buf()
                .await
                .map_err(|error| format!("failed to read agent stdout: {error}"))?;
            if available.is_empty() {
                if bytes == 0 {
                    return Ok(None);
                }
                if !oversize {
                    self.record_line(&line);
                }
                return Ok(Some(classify_line(
                    line,
                    bytes,
                    oversize,
                    oversize_kind_hint,
                )));
            }

            let newline = available.iter().position(|byte| *byte == b'\n');
            let content_bytes = newline.unwrap_or(available.len());

            let next_bytes = bytes.saturating_add(content_bytes);
            if !oversize && next_bytes <= self.max_line_bytes {
                line.extend_from_slice(&available[..content_bytes]);
            } else if !oversize {
                oversize = true;
                let remaining_probe_bytes = LINE_PROBE_BYTES.saturating_sub(line.len());
                line.extend_from_slice(&available[..content_bytes.min(remaining_probe_bytes)]);
                oversize_kind_hint = infer_kind_hint(&line[..line.len().min(LINE_PROBE_BYTES)]);
                line.clear();
            }
            bytes = next_bytes;

            let consumed = content_bytes + usize::from(newline.is_some());
            self.inner.consume(consumed);
            if newline.is_some() {
                if !oversize {
                    self.record_line(&line);
                }
                return Ok(Some(classify_line(
                    line,
                    bytes,
                    oversize,
                    oversize_kind_hint,
                )));
            }
        }
    }

    fn record_line(&self, line: &[u8]) {
        if let Some(wire_recorder) = &self.wire_recorder {
            wire_recorder.record(line.to_vec());
        }
    }
}

fn classify_line(
    line: Vec<u8>,
    bytes: usize,
    oversize: bool,
    oversize_kind_hint: Option<String>,
) -> StdoutItem {
    if oversize {
        return StdoutItem::Oversize {
            probe: LineProbe {
                kind_hint: oversize_kind_hint,
                bytes,
            },
        };
    }

    match serde_json::from_slice::<Value>(&line) {
        Ok(value) => StdoutItem::Json(value),
        Err(_) => StdoutItem::NonJson {
            probe: LineProbe {
                kind_hint: None,
                bytes,
            },
        },
    }
}

fn infer_kind_hint(prefix: &[u8]) -> Option<String> {
    find_json_string_field(prefix, b"\"type\"")
        .or_else(|| find_json_string_field(prefix, b"\"method\""))
}

fn find_json_string_field(prefix: &[u8], key: &[u8]) -> Option<String> {
    let mut offset = 0;
    while offset + key.len() <= prefix.len() {
        if &prefix[offset..offset + key.len()] != key {
            offset += 1;
            continue;
        }

        let mut cursor = offset + key.len();
        skip_ascii_whitespace(prefix, &mut cursor);
        if prefix.get(cursor) != Some(&b':') {
            offset += 1;
            continue;
        }
        cursor += 1;
        skip_ascii_whitespace(prefix, &mut cursor);
        if prefix.get(cursor) != Some(&b'"') {
            offset += 1;
            continue;
        }

        let value_start = cursor;
        cursor += 1;
        let mut escaped = false;
        while let Some(byte) = prefix.get(cursor).copied() {
            if !escaped && byte == b'"' {
                if cursor.saturating_sub(value_start) > MAX_KIND_HINT_BYTES {
                    return None;
                }
                return serde_json::from_slice::<String>(&prefix[value_start..=cursor]).ok();
            }
            escaped = !escaped && byte == b'\\';
            cursor += 1;
        }
        return None;
    }
    None
}

fn skip_ascii_whitespace(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncBufRead, AsyncRead, BufReader, ReadBuf};

    use super::*;
    use crate::infrastructure::agent_session::wire_record::{WireBackend, WireRecorder};

    #[tokio::test]
    async fn test_stdout_line_reader_json行を分類する() {
        let input = b"{\"ok\":true}\n";
        let mut reader = StdoutLineReader::new(BufReader::new(&input[..]));

        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Json(value)) if value["ok"] == serde_json::json!(true)
        ));
        assert!(reader.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_stdout_line_reader非json行を通知し後続jsonを処理する() {
        let input = b"warning from injected environment\n{\"ok\":true}\n";
        let mut reader = StdoutLineReader::new(BufReader::new(&input[..]));

        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::NonJson { probe }) if probe.bytes == 33 && probe.kind_hint.is_none()
        ));
        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Json(value)) if value["ok"] == serde_json::json!(true)
        ));
    }

    #[tokio::test]
    async fn test_stdout_line_readerは正常行と非json生行を記録する() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = WireRecorder::for_test(dir.path().to_path_buf(), WireBackend::Claude);
        let input = b"not-json\n{\"type\":\"result\"}\n";
        let mut reader = StdoutLineReader::with_wire_recorder(BufReader::new(&input[..]), recorder);

        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::NonJson { .. })
        ));
        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Json(value)) if value == serde_json::json!({"type": "result"})
        ));
        assert!(reader.next().await.unwrap().is_none());

        reader.shutdown_wire_recorder().await;
        assert_eq!(
            std::fs::read_to_string(dir.path().join("claude.jsonl")).unwrap(),
            "not-json\n{\"type\":\"result\"}\n"
        );
    }

    #[tokio::test]
    async fn test_stdout_line_reader上限超過行を保持せず後続jsonを処理する() {
        let payload = "x".repeat(65);
        let input = format!("{{\"type\":\"assistant\",\"data\":\"{payload}\"}}\n{{\"ok\":true}}\n");
        let mut reader = StdoutLineReader::with_max_line_bytes(
            BufReader::with_capacity(16, input.as_bytes()),
            64,
        );

        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Oversize { probe })
                if probe.bytes > 64 && probe.kind_hint.as_deref() == Some("assistant")
        ));
        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Json(value)) if value["ok"] == serde_json::json!(true)
        ));
    }

    #[tokio::test]
    async fn test_stdout_line_reader本番8mb上限を超える行を破棄する() {
        let payload = "x".repeat(MAX_STDOUT_LINE_BYTES);
        let input = format!(
            "{{\"method\":\"item/agentMessage/delta\",\"data\":\"{payload}\"}}\n{{\"ok\":true}}\n"
        );
        let mut reader =
            StdoutLineReader::new(BufReader::with_capacity(64 * 1024, input.as_bytes()));

        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Oversize { probe })
                if probe.bytes > MAX_STDOUT_LINE_BYTES
                    && probe.kind_hint.as_deref() == Some("item/agentMessage/delta")
        ));
        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Json(value)) if value["ok"] == serde_json::json!(true)
        ));
    }

    #[tokio::test]
    async fn test_stdout_line_reader上限ちょうどの行は破棄しない() {
        let input = b"0123456789";
        let mut reader =
            StdoutLineReader::with_max_line_bytes(BufReader::new(&input[..]), input.len());

        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::NonJson { probe }) if probe.bytes == input.len()
        ));
    }

    #[tokio::test]
    async fn test_stdout_line_reader改行なしeofの超過行も通知する() {
        let input = b"01234567890";
        let mut reader = StdoutLineReader::with_max_line_bytes(BufReader::new(&input[..]), 10);

        assert!(matches!(
            reader.next().await.unwrap(),
            Some(StdoutItem::Oversize { probe }) if probe.bytes == input.len()
        ));
        assert!(reader.next().await.unwrap().is_none());
    }

    #[test]
    fn test_stdout_line_reader共通上限は8mb() {
        assert_eq!(MAX_STDOUT_LINE_BYTES, 8 * 1024 * 1024);
    }

    #[test]
    fn test_stdout_diagnosticsは共通カウントと上限文言を生成してresetする() {
        let mut diagnostics = StdoutDiagnostics::default();
        let probe = LineProbe {
            kind_hint: Some("assistant".to_string()),
            bytes: 9 * 1024 * 1024,
        };

        diagnostics.record_non_json_skip("test", &probe);
        let message = diagnostics.record_oversize_drop("test", &probe);

        assert_eq!(diagnostics.skipped_non_json_count(), 1);
        assert_eq!(diagnostics.oversize_dropped_count(), 1);
        assert_eq!(
            message,
            "backend からの応答 1 件がサイズ上限（8MB）を超えたため破棄しました（推定種別: assistant）"
        );

        diagnostics.reset();
        assert_eq!(diagnostics.skipped_non_json_count(), 0);
        assert_eq!(diagnostics.oversize_dropped_count(), 0);
    }

    #[tokio::test]
    async fn test_stdout_line_readerioエラーのみをerrとして返す() {
        let mut reader = StdoutLineReader::new(FailingReader);

        assert!(matches!(
            reader.next().await,
            Err(message) if message.contains("injected read failure")
        ));
    }

    struct FailingReader;

    impl AsyncRead for FailingReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(std::io::Error::other("injected read failure")))
        }
    }

    impl AsyncBufRead for FailingReader {
        fn poll_fill_buf(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<&[u8]>> {
            Poll::Ready(Err(std::io::Error::other("injected read failure")))
        }

        fn consume(self: Pin<&mut Self>, _amt: usize) {}
    }
}
