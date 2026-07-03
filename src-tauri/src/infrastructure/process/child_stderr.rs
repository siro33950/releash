use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub(crate) async fn drain_child_stderr(label: &'static str, stderr: impl AsyncRead + Unpin) {
    let mut lines = BufReader::new(stderr).lines();
    let mut line_count: u64 = 0;
    let mut byte_count: u64 = 0;
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                line_count = line_count.saturating_add(1);
                byte_count = byte_count.saturating_add(line.len() as u64);
            }
            Ok(None) => break,
            Err(error) => {
                log::warn!("failed to drain {label} stderr: {error}");
                break;
            }
        }
    }
    if line_count > 0 {
        log::debug!("{label} stderr drained: lines={line_count} bytes={byte_count}");
    }
}
