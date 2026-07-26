use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::Value;

use super::{ActivityEntry, ChatMessage, MessagePart, MessageRole};

pub(crate) fn agent_read_paths_from_messages(messages: &[ChatMessage]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for message in messages
        .iter()
        .filter(|message| message.role == MessageRole::Agent)
    {
        if let Some(parts) = message.parts.as_ref() {
            for part in parts {
                if let MessagePart::ToolUse { tool, input, .. } = part {
                    let input: Value = serde_json::from_str(input.as_str())
                        .expect("domain JsonPayload must be validated at its boundary");
                    push_tool_read_paths(tool, &input, &mut seen, &mut paths);
                }
            }
        }
        if let Some(activities) = message.activities.as_ref() {
            for activity in activities {
                if let ActivityEntry::ToolUse { tool, input, .. } = activity {
                    push_tool_read_paths(tool, input, &mut seen, &mut paths);
                }
            }
        }
    }
    paths
}

#[cfg(test)]
pub(crate) fn agent_read_paths_from_message(message: &ChatMessage) -> Vec<PathBuf> {
    agent_read_paths_from_messages(std::slice::from_ref(message))
}

#[cfg(test)]
pub(crate) fn agent_read_paths_from_parts(parts: &[MessagePart]) -> Vec<PathBuf> {
    let message = ChatMessage {
        id: String::new(),
        role: MessageRole::Agent,
        content: String::new(),
        thinking: None,
        activities: None,
        parts: Some(parts.to_vec()),
        streaming_final_seq: 0,
        timestamp: 0.0,
        mentions: None,
    };
    agent_read_paths_from_message(&message)
}

#[cfg(test)]
pub(crate) fn merge_agent_read_paths(
    cache: &mut Option<Vec<PathBuf>>,
    new_paths: impl IntoIterator<Item = PathBuf>,
) {
    let paths = cache.get_or_insert_with(Vec::new);
    let mut seen = paths.iter().cloned().collect::<HashSet<_>>();
    for path in new_paths {
        if seen.insert(path.clone()) {
            paths.push(path);
        }
    }
}

fn push_tool_read_paths(
    tool: &str,
    input: &Value,
    seen: &mut HashSet<PathBuf>,
    paths: &mut Vec<PathBuf>,
) {
    for path in tool_read_paths(tool, input) {
        if seen.insert(path.clone()) {
            paths.push(path);
        }
    }
}

fn tool_read_paths(tool: &str, input: &Value) -> Vec<PathBuf> {
    let tool_name = tool.to_ascii_lowercase();
    let mut paths = Vec::new();
    if matches!(
        tool_name.as_str(),
        "read" | "readfile" | "read_file" | "view" | "open"
    ) {
        collect_json_path_values(input, &mut paths);
    }
    if matches!(
        tool_name.as_str(),
        "bash" | "shell" | "exec" | "exec_command" | "command"
    ) {
        if let Some(command) = input
            .get("command")
            .or_else(|| input.get("cmd"))
            .and_then(Value::as_str)
        {
            paths.extend(shell_read_paths(command));
        }
    }
    paths
}

fn collect_json_path_values(input: &Value, paths: &mut Vec<PathBuf>) {
    const PATH_KEYS: [&str; 5] = ["file_path", "filepath", "path", "relative_path", "file"];
    match input {
        Value::Object(map) => {
            for (key, value) in map {
                if PATH_KEYS.contains(&key.as_str()) {
                    collect_path_value(value, paths);
                } else if matches!(value, Value::Object(_) | Value::Array(_)) {
                    collect_json_path_values(value, paths);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_json_path_values(value, paths);
            }
        }
        _ => {}
    }
}

fn collect_path_value(value: &Value, paths: &mut Vec<PathBuf>) {
    match value {
        Value::String(path) if looks_like_read_path(path) => paths.push(PathBuf::from(path)),
        Value::Array(values) => {
            for value in values {
                collect_path_value(value, paths);
            }
        }
        _ => {}
    }
}

fn shell_read_paths(command: &str) -> Vec<PathBuf> {
    let tokens = command
        .split_whitespace()
        .map(|token| token.trim_matches(|c| matches!(c, '\'' | '"' | '`')))
        .collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut read_command = false;
    for token in tokens {
        let command_name = token.rsplit('/').next().unwrap_or(token);
        if matches!(
            command_name,
            "cat" | "less" | "more" | "nl" | "sed" | "head" | "tail"
        ) {
            read_command = true;
            continue;
        }
        if matches!(token, "|" | "||" | "&&" | ";") {
            read_command = false;
            continue;
        }
        if read_command && looks_like_read_path(token) {
            paths.push(PathBuf::from(token));
        }
    }
    paths
}

fn looks_like_read_path(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && !path.starts_with('-')
        && !path.contains('\n')
        && (path.contains('/') || path.contains('.') || path.starts_with('~'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_read_paths_extracts_read_tool_and_shell_reads() {
        let messages = vec![ChatMessage {
            id: "agent-1".to_string(),
            role: MessageRole::Agent,
            content: String::new(),
            thinking: None,
            activities: None,
            parts: Some(vec![
                MessagePart::ToolUse {
                    tool: "Read".to_string(),
                    input: serde_json::json!({"file_path": "src/foo/bar.rs"}).into(),
                    id: "tool-1".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::ToolUse {
                    tool: "Bash".to_string(),
                    input: serde_json::json!({"command": "sed -n '1,80p' src/foo/baz.rs && cat Cargo.toml"}).into(),
                    id: "tool-2".to_string(),
                    parent_tool_use_id: None,
                },
            ]),
            streaming_final_seq: 0,
            timestamp: 1.0,
            mentions: None,
        }];

        let paths = agent_read_paths_from_messages(&messages);

        assert!(paths.contains(&PathBuf::from("src/foo/bar.rs")));
        assert!(paths.contains(&PathBuf::from("src/foo/baz.rs")));
        assert!(paths.contains(&PathBuf::from("Cargo.toml")));
    }
}
