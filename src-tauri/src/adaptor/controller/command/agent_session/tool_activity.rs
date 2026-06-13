#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolActivityPresentation {
    pub category: String,
    pub label: String,
    pub summary: String,
    pub edit_preview_tool: bool,
}

fn classify_tool(tool_name: &str) -> &'static str {
    match tool_name {
        "Read"
        | "Glob"
        | "Grep"
        | "WebFetch"
        | "WebSearch"
        | "ListMcpResourcesTool"
        | "ReadMcpResourceTool"
        | "ToolSearch" => "read",
        "Bash" => "command",
        "Write" | "Edit" | "NotebookEdit" => "write",
        _ if tool_name.starts_with("mcp__") => classify_mcp_tool(tool_name),
        _ => "other",
    }
}

fn classify_mcp_tool(tool_name: &str) -> &'static str {
    let lower = tool_name.to_lowercase();
    if [
        "read", "get", "list", "search", "fetch", "retrieve", "query",
    ]
    .iter()
    .any(|token| lower.contains(token))
    {
        return "read";
    }
    if [
        "write", "create", "update", "delete", "edit", "post", "patch", "put",
    ]
    .iter()
    .any(|token| lower.contains(token))
    {
        return "write";
    }
    "other"
}

fn shorten_path(full_path: &str, base_path: Option<&str>) -> String {
    let Some(base_path) = base_path else {
        return full_path.to_string();
    };
    if full_path == base_path {
        return ".".to_string();
    }
    let prefix = format!("{base_path}/");
    if let Some(relative) = full_path.strip_prefix(&prefix) {
        return relative.to_string();
    }
    full_path.to_string()
}

fn string_input<'a>(input: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn read_tool_label(tool_name: &str, input: &serde_json::Value, base_path: Option<&str>) -> String {
    if let Some(file_path) = string_input(input, "file_path") {
        return format!("Explored {}", shorten_path(file_path, base_path));
    }
    if let Some(pattern) = string_input(input, "pattern") {
        return format!("Explored {pattern}");
    }
    if let Some(path) = string_input(input, "path") {
        return format!("Explored {}", shorten_path(path, base_path));
    }
    if let Some(query) = string_input(input, "query") {
        return format!("Searched \"{}\"", truncate_chars(query, 60));
    }
    if let Some(url) = string_input(input, "url") {
        return format!("Fetched {url}");
    }
    format!("Explored ({tool_name})")
}

fn command_tool_label(input: &serde_json::Value) -> String {
    string_input(input, "command")
        .map(|command| truncate_chars(command, 80))
        .unwrap_or_else(|| "command".to_string())
}

fn default_tool_summary(
    tool_name: &str,
    input: &serde_json::Value,
    base_path: Option<&str>,
) -> String {
    if let Some(file_path) = string_input(input, "file_path") {
        return shorten_path(file_path, base_path);
    }
    if let Some(pattern) = string_input(input, "pattern") {
        return pattern.to_string();
    }
    if let Some(command) = string_input(input, "command") {
        return truncate_chars(command, 80);
    }
    let Some(object) = input.as_object() else {
        return tool_name.to_string();
    };
    let Some((first_key, value)) = object.iter().next() else {
        return tool_name.to_string();
    };
    if let Some(value) = value.as_str() {
        return truncate_chars(value, 60);
    }
    format!("{first_key}: ...")
}

fn is_edit_preview_tool(tool_name: &str) -> bool {
    matches!(tool_name, "Edit" | "MultiEdit" | "Write")
}

pub(crate) fn present_agent_tool_activity_inner(
    tool_name: &str,
    input: &serde_json::Value,
    base_path: Option<&str>,
) -> AgentToolActivityPresentation {
    let category = classify_tool(tool_name);
    let label = match category {
        "read" => read_tool_label(tool_name, input, base_path),
        "command" => command_tool_label(input),
        _ => default_tool_summary(tool_name, input, base_path),
    };
    AgentToolActivityPresentation {
        category: category.to_string(),
        summary: default_tool_summary(tool_name, input, base_path),
        label,
        edit_preview_tool: is_edit_preview_tool(tool_name),
    }
}

#[tauri::command]
pub fn present_agent_tool_activity(
    tool_name: String,
    input: serde_json::Value,
    base_path: Option<String>,
) -> AgentToolActivityPresentation {
    present_agent_tool_activity_inner(&tool_name, &input, base_path.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_native_tools() {
        assert_eq!(classify_tool("Read"), "read");
        assert_eq!(classify_tool("Bash"), "command");
        assert_eq!(classify_tool("Write"), "write");
        assert_eq!(classify_tool("Unknown"), "other");
    }

    #[test]
    fn classifies_mcp_tools_by_name_pattern() {
        assert_eq!(classify_tool("mcp__notion__get_page"), "read");
        assert_eq!(classify_tool("mcp__server__search_docs"), "read");
        assert_eq!(classify_tool("mcp__server__create_page"), "write");
        assert_eq!(classify_tool("mcp__server__patch_item"), "write");
        assert_eq!(classify_tool("mcp__server__run_something"), "other");
    }

    #[test]
    fn presents_read_labels_and_shortens_base_path() {
        let result = present_agent_tool_activity_inner(
            "Read",
            &serde_json::json!({"file_path": "/repo/src/main.ts"}),
            Some("/repo"),
        );

        assert_eq!(result.category, "read");
        assert_eq!(result.label, "Explored src/main.ts");
    }

    #[test]
    fn presents_command_labels_with_truncation() {
        let command = "a".repeat(100);
        let result = present_agent_tool_activity_inner(
            "Bash",
            &serde_json::json!({ "command": command }),
            None,
        );

        assert_eq!(result.category, "command");
        assert_eq!(result.label.len(), 83);
        assert!(result.label.ends_with("..."));
    }

    #[test]
    fn presents_default_summary_and_edit_preview_flag() {
        let result = present_agent_tool_activity_inner(
            "Edit",
            &serde_json::json!({"file_path": "/repo/src/app.ts"}),
            Some("/repo"),
        );

        assert_eq!(result.category, "write");
        assert_eq!(result.summary, "src/app.ts");
        assert!(result.edit_preview_tool);
    }
}
