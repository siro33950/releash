use serde_json::Value;
use tokio::io::AsyncBufRead;

use crate::infrastructure::agent_session::codex::app_server::CodexAppServerProcess;
use crate::infrastructure::agent_session::codex::wire::{
    initialize_request, initialized_notification, request, PendingClientRequests, METHOD_INITIALIZE,
};
use crate::infrastructure::agent_session::stdout_line_reader::{
    StdoutDiagnostics, StdoutItem, StdoutLineReader,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexOneShotError {
    Timeout,
    External(String),
}

impl From<String> for CodexOneShotError {
    fn from(error: String) -> Self {
        Self::External(error)
    }
}

impl std::fmt::Display for CodexOneShotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("codex one-shot request timed out"),
            Self::External(message) => formatter.write_str(message),
        }
    }
}

pub(crate) async fn request_once(
    cli_path: &str,
    session_id: &str,
    cwd: Option<&str>,
    method: &str,
    params: Value,
) -> Result<Value, CodexOneShotError> {
    let mut process = CodexAppServerProcess::spawn(cli_path, session_id, cwd, None, &[]).await?;
    let handle = process.handle();
    if let Err(error) = handle
        .write_json(&initialize_request(1, env!("CARGO_PKG_VERSION")))
        .await
    {
        process.shutdown().await;
        return Err(error.into());
    }
    if let Err(error) = handle.write_json(&initialized_notification()).await {
        process.shutdown().await;
        return Err(error.into());
    }
    if let Err(error) = handle.write_json(&request(2, method, params)).await {
        process.shutdown().await;
        return Err(error.into());
    }

    let mut pending_requests = PendingClientRequests::default();
    pending_requests.register(1, METHOD_INITIALIZE);
    pending_requests.register(2, method);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        read_response(process.stdout_mut(), pending_requests, 2),
    )
    .await;
    process.shutdown().await;
    response
        .map_err(|_| CodexOneShotError::Timeout)?
        .map_err(CodexOneShotError::External)
}

async fn read_response<R>(
    stdout: &mut StdoutLineReader<R>,
    mut pending_requests: PendingClientRequests,
    expected_id: u64,
) -> Result<Value, String>
where
    R: AsyncBufRead + Unpin,
{
    let mut diagnostics = StdoutDiagnostics::default();
    loop {
        let Some(item) = stdout.next().await? else {
            return Err("codex app-server exited before one-shot response".to_string());
        };
        let message = match item {
            StdoutItem::Json(message) => message,
            StdoutItem::NonJson { probe } => {
                diagnostics.record_non_json_skip("codex one-shot", &probe);
                continue;
            }
            StdoutItem::Oversize { probe } => {
                let _ = diagnostics.record_oversize_drop("codex one-shot", &probe);
                continue;
            }
        };
        let Some(response) = pending_requests.take_response(&message)? else {
            continue;
        };
        if let Some(error) = message.get("error") {
            return Err(error.to_string());
        }
        if response.id != expected_id {
            continue;
        }
        return Ok(message
            .get("result")
            .cloned()
            .expect("validated JSON-RPC response must contain result or error"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::agent_session::codex::wire::METHOD_SKILLS_LIST;
    use serde_json::json;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn response_requires_result_or_error() {
        let input = br#"{"id":2}
"#;
        let mut stdout = StdoutLineReader::new(BufReader::new(&input[..]));
        let mut pending = PendingClientRequests::default();
        pending.register(2, METHOD_SKILLS_LIST);

        let result = read_response(&mut stdout, pending, 2).await;

        assert!(matches!(
            result,
            Err(message) if message.contains("expected exactly one of result or error")
        ));
    }

    #[tokio::test]
    async fn initialize_error_precedes_expected_response() {
        let input = br#"{"id":1,"error":{"code":-32603,"message":"initialize failed"}}
{"id":2,"result":{"skills":[]}}
"#;
        let mut stdout = StdoutLineReader::new(BufReader::new(&input[..]));
        let mut pending = PendingClientRequests::default();
        pending.register(1, METHOD_INITIALIZE);
        pending.register(2, METHOD_SKILLS_LIST);

        let result = read_response(&mut stdout, pending, 2).await;

        assert_eq!(
            result.unwrap_err(),
            json!({ "code": -32603, "message": "initialize failed" }).to_string()
        );
    }

    #[tokio::test]
    async fn diagnostics_are_skipped_before_the_response() {
        let payload = "x".repeat(64);
        let input = format!(
            "diagnostic output\n{{\"method\":\"ignored\",\"params\":{{\"data\":\"{payload}\"}}}}\n{{\"id\":2,\"result\":{{\"skills\":[]}}}}\n"
        );
        let mut stdout =
            StdoutLineReader::with_max_line_bytes(BufReader::new(input.as_bytes()), 48);
        let mut pending = PendingClientRequests::default();
        pending.register(2, METHOD_SKILLS_LIST);

        let result = read_response(&mut stdout, pending, 2).await;

        assert_eq!(result.unwrap(), json!({ "skills": [] }));
    }
}
