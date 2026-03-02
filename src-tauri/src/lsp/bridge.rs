use bytes::BytesMut;
use tauri::ipc::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, ChildStdout};

use super::LspMessage;

const CONTENT_LENGTH_HEADER: &str = "Content-Length: ";
const HEADER_SEPARATOR: &[u8] = b"\r\n\r\n";

/// Encode a JSON message with Content-Length header for LSP protocol.
pub fn encode(json: &[u8]) -> Vec<u8> {
    let header = format!("{CONTENT_LENGTH_HEADER}{}\r\n\r\n", json.len());
    let mut buf = Vec::with_capacity(header.len() + json.len());
    buf.extend_from_slice(header.as_bytes());
    buf.extend_from_slice(json);
    buf
}

/// Decode a single LSP message from the buffer.
/// Returns `Some(json_bytes)` if a complete message is available, `None` otherwise.
pub fn decode(buf: &mut BytesMut) -> Option<Vec<u8>> {
    // Find the header separator
    let sep_pos = buf
        .windows(HEADER_SEPARATOR.len())
        .position(|w| w == HEADER_SEPARATOR)?;

    let header = std::str::from_utf8(&buf[..sep_pos]).ok()?;

    // Parse Content-Length
    let content_length = header
        .split("\r\n")
        .find_map(|line| line.strip_prefix(CONTENT_LENGTH_HEADER))
        .and_then(|v| v.trim().parse::<usize>().ok())?;

    let body_start = sep_pos + HEADER_SEPARATOR.len();
    let total_len = body_start + content_length;

    if buf.len() < total_len {
        return None; // Incomplete message
    }

    let body = buf[body_start..total_len].to_vec();
    let _ = buf.split_to(total_len);
    Some(body)
}

/// Write a JSON-RPC message to LSP server's stdin with Content-Length framing.
pub async fn write_to_stdin(stdin: &mut ChildStdin, json: &str) -> Result<(), String> {
    let encoded = encode(json.as_bytes());
    stdin
        .write_all(&encoded)
        .await
        .map_err(|e| format!("LSP stdin書き込み失敗: {e}"))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("LSP stdin flush失敗: {e}"))?;
    Ok(())
}

/// Spawn an async task that reads LSP messages from stdout and sends them via Tauri Channel.
pub fn spawn_stdout_reader(session_id: u64, stdout: ChildStdout, channel: Channel<LspMessage>) {
    tokio::spawn(async move {
        if let Err(e) = read_stdout_loop(session_id, stdout, channel).await {
            log::debug!("LSP[{session_id}] stdout reader ended: {e}");
        }
    });
}

async fn read_stdout_loop(
    session_id: u64,
    stdout: ChildStdout,
    channel: Channel<LspMessage>,
) -> Result<(), String> {
    let mut buf = BytesMut::with_capacity(8192);
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut read_buf = [0u8; 4096];

    loop {
        let n = reader
            .read(&mut read_buf)
            .await
            .map_err(|e| format!("LSP stdout読み取り失敗: {e}"))?;

        if n == 0 {
            return Ok(()); // EOF — process has exited
        }

        buf.extend_from_slice(&read_buf[..n]);

        // Decode as many complete messages as available
        while let Some(body) = decode(&mut buf) {
            match String::from_utf8(body) {
                Ok(json_str) => {
                    let _ = channel.send(LspMessage {
                        session_id,
                        message: json_str,
                    });
                }
                Err(e) => {
                    log::warn!("LSP[{session_id}] non-UTF-8 message: {e}");
                }
            }
        }
    }
}

/// Intercept `initialize` requests and inject `rootUri` from the worktree path.
/// If the message is not an `initialize` request, it is returned unchanged.
pub fn inject_root_uri(message: &str, worktree_path: &str) -> Result<String, String> {
    let mut json: serde_json::Value =
        serde_json::from_str(message).map_err(|e| format!("JSONパース失敗: {e}"))?;

    let is_initialize = json
        .get("method")
        .and_then(|m| m.as_str())
        .is_some_and(|m| m == "initialize");

    if !is_initialize {
        return Ok(message.to_string());
    }

    if let Some(params) = json.get_mut("params") {
        let uri = format!("file://{}", worktree_path.replace(' ', "%20"));
        params["rootUri"] = serde_json::Value::String(uri.clone());

        // Also set workspaceFolders if not provided
        if params.get("workspaceFolders").is_none() || params["workspaceFolders"].is_null() {
            params["workspaceFolders"] = serde_json::json!([{
                "uri": uri,
                "name": std::path::Path::new(worktree_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("workspace")
            }]);
        }
    }

    serde_json::to_string(&json).map_err(|e| format!("JSONシリアライズ失敗: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_creates_valid_content_length_header() {
        let json = b"{\"jsonrpc\":\"2.0\"}";
        let encoded = encode(json);
        let expected = format!(
            "Content-Length: {}\r\n\r\n{}",
            json.len(),
            "{\"jsonrpc\":\"2.0\"}"
        );
        assert_eq!(encoded, expected.as_bytes());
    }

    #[test]
    fn decode_parses_complete_message() {
        let json = b"{\"jsonrpc\":\"2.0\",\"id\":1}";
        let raw = encode(json);
        let mut buf = BytesMut::from(&raw[..]);

        let result = decode(&mut buf);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), json.to_vec());
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_returns_none_for_incomplete_header() {
        let mut buf = BytesMut::from("Content-Length: 10\r\n");
        assert!(decode(&mut buf).is_none());
    }

    #[test]
    fn decode_returns_none_for_incomplete_body() {
        let mut buf = BytesMut::from("Content-Length: 100\r\n\r\n{\"short\"}");
        assert!(decode(&mut buf).is_none());
    }

    #[test]
    fn decode_handles_multiple_messages() {
        let msg1 = b"{\"id\":1}";
        let msg2 = b"{\"id\":2}";
        let mut raw = encode(msg1);
        raw.extend_from_slice(&encode(msg2));

        let mut buf = BytesMut::from(&raw[..]);

        let r1 = decode(&mut buf);
        assert_eq!(r1.unwrap(), msg1.to_vec());

        let r2 = decode(&mut buf);
        assert_eq!(r2.unwrap(), msg2.to_vec());

        assert!(buf.is_empty());
    }

    #[test]
    fn decode_handles_extra_headers() {
        let json = b"{\"ok\":true}";
        let raw = format!(
            "Content-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            json.len(),
            std::str::from_utf8(json).unwrap()
        );
        let mut buf = BytesMut::from(raw.as_bytes());

        let result = decode(&mut buf);
        assert_eq!(result.unwrap(), json.to_vec());
    }

    #[test]
    fn inject_root_uri_on_initialize() {
        let message = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"rootUri":null,"capabilities":{}}}"#;
        let result = inject_root_uri(message, "/path/to/project").unwrap();
        let json: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(json["params"]["rootUri"], "file:///path/to/project");
        assert!(json["params"]["workspaceFolders"].is_array());
        assert_eq!(
            json["params"]["workspaceFolders"][0]["uri"],
            "file:///path/to/project"
        );
        assert_eq!(json["params"]["workspaceFolders"][0]["name"], "project");
    }

    #[test]
    fn inject_root_uri_passes_non_initialize() {
        let message = r#"{"jsonrpc":"2.0","id":1,"method":"textDocument/completion","params":{}}"#;
        let result = inject_root_uri(message, "/path/to/project").unwrap();
        assert_eq!(result, message);
    }

    #[test]
    fn inject_root_uri_escapes_spaces() {
        let message = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"rootUri":null}}"#;
        let result = inject_root_uri(message, "/path/to/my project").unwrap();
        let json: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["params"]["rootUri"], "file:///path/to/my%20project");
    }

    #[test]
    fn inject_root_uri_preserves_existing_workspace_folders() {
        let message = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"rootUri":null,"workspaceFolders":[{"uri":"file:///custom","name":"custom"}]}}"#;
        let result = inject_root_uri(message, "/path/to/project").unwrap();
        let json: serde_json::Value = serde_json::from_str(&result).unwrap();
        // rootUri is still replaced
        assert_eq!(json["params"]["rootUri"], "file:///path/to/project");
        // But existing workspaceFolders are preserved
        assert_eq!(
            json["params"]["workspaceFolders"][0]["uri"],
            "file:///custom"
        );
    }
}
