use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEditPreview {
    pub tool_name: String,
    pub operation: String,
    pub file_path: Option<String>,
    pub hunks: Vec<AgentEditPreviewHunk>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEditPreviewHunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<AgentEditPreviewLine>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEditPreviewLine {
    pub kind: AgentEditPreviewLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentEditPreviewLineKind {
    Context,
    Removed,
    Added,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MultiEditOperation {
    #[serde(default)]
    old_string: String,
    #[serde(default)]
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

fn input_str<'a>(input: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(|value| value.as_str())
}

fn has_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
}

fn has_forbidden_relative_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    })
}

fn resolve_tool_file_path(worktree_path: &str, file_path: &str) -> Result<PathBuf, String> {
    if file_path.trim().is_empty() {
        return Err("file_path is empty".to_string());
    }

    let root = Path::new(worktree_path)
        .canonicalize()
        .map_err(|e| format!("Failed to resolve worktree path: {e}"))?;
    let raw_path = Path::new(file_path);

    if raw_path.is_absolute() {
        if has_parent_component(raw_path) {
            return Err("file_path must not contain parent directory components".to_string());
        }
        match raw_path.canonicalize() {
            Ok(canonical) if canonical.starts_with(&root) => Ok(canonical),
            Ok(_) => Err("file_path resolves outside the worktree".to_string()),
            Err(_) if raw_path.starts_with(&root) => Ok(raw_path.to_path_buf()),
            Err(e) => Err(format!(
                "file_path is outside the worktree or unreadable: {e}"
            )),
        }
    } else {
        if has_forbidden_relative_component(raw_path) {
            return Err("file_path must be relative to the worktree without traversal".to_string());
        }
        Ok(root.join(raw_path))
    }
}

fn read_existing_file(path: &Path) -> Result<(String, Vec<String>), String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok((content, Vec::new())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((
            String::new(),
            vec!["Existing file was not found; preview is shown as a new file.".to_string()],
        )),
        Err(e) => Err(format!("Failed to read existing file: {e}")),
    }
}

fn apply_edit(original: &str, old_string: &str, new_string: &str, replace_all: bool) -> String {
    if old_string.is_empty() {
        return original.to_string();
    }
    if replace_all {
        original.replace(old_string, new_string)
    } else {
        original.replacen(old_string, new_string, 1)
    }
}

fn line_from_prefixed(
    line: &str,
    old_line: &mut u32,
    new_line: &mut u32,
) -> Option<AgentEditPreviewLine> {
    let (kind, content) = line.split_at(1);
    match kind {
        " " => {
            let result = AgentEditPreviewLine {
                kind: AgentEditPreviewLineKind::Context,
                old_line: Some(*old_line),
                new_line: Some(*new_line),
                content: content.to_string(),
            };
            *old_line += 1;
            *new_line += 1;
            Some(result)
        }
        "-" => {
            let result = AgentEditPreviewLine {
                kind: AgentEditPreviewLineKind::Removed,
                old_line: Some(*old_line),
                new_line: None,
                content: content.to_string(),
            };
            *old_line += 1;
            Some(result)
        }
        "+" => {
            let result = AgentEditPreviewLine {
                kind: AgentEditPreviewLineKind::Added,
                old_line: None,
                new_line: Some(*new_line),
                content: content.to_string(),
            };
            *new_line += 1;
            Some(result)
        }
        _ => None,
    }
}

fn build_hunks(
    file_path: Option<&str>,
    original: &str,
    modified: &str,
) -> Vec<AgentEditPreviewHunk> {
    crate::adaptor::gateway::code::diff_compute::diff_buffers(original, modified, file_path)
        .into_iter()
        .map(|hunk| {
            let mut old_line = hunk.old_start;
            let mut new_line = hunk.new_start;
            let lines = hunk
                .lines
                .iter()
                .filter_map(|line| line_from_prefixed(line, &mut old_line, &mut new_line))
                .collect();
            AgentEditPreviewHunk {
                old_start: hunk.old_start,
                new_start: hunk.new_start,
                lines,
            }
        })
        .collect()
}

fn build_agent_edit_preview_inner(
    worktree_path: &str,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<Option<AgentEditPreview>, String> {
    let tool = tool_name.trim();
    if !matches!(tool, "Edit" | "MultiEdit" | "Write") {
        return Ok(None);
    }

    let Some(file_path) = input_str(input, "file_path") else {
        return Ok(Some(AgentEditPreview {
            tool_name: tool.to_string(),
            operation: "Missing file path".to_string(),
            file_path: None,
            hunks: Vec::new(),
            warnings: vec!["Tool input did not include file_path.".to_string()],
        }));
    };

    let resolved_path = resolve_tool_file_path(worktree_path, file_path)?;
    let (original, mut warnings) = read_existing_file(&resolved_path)?;

    let (modified, operation) = match tool {
        "Write" => (
            input_str(input, "content").unwrap_or_default().to_string(),
            "Write file".to_string(),
        ),
        "Edit" => {
            let old_string = input_str(input, "old_string").unwrap_or_default();
            let new_string = input_str(input, "new_string").unwrap_or_default();
            let replace_all = input
                .get("replace_all")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if old_string.is_empty() {
                warnings.push("Edit input did not include old_string.".to_string());
            } else if !original.contains(old_string) {
                warnings.push("old_string was not found in the current file.".to_string());
            }
            (
                apply_edit(&original, old_string, new_string, replace_all),
                if replace_all {
                    "Edit all matches".to_string()
                } else {
                    "Edit first match".to_string()
                },
            )
        }
        "MultiEdit" => {
            let edits: Vec<MultiEditOperation> = input
                .get("edits")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| format!("Invalid MultiEdit edits: {e}"))?
                .unwrap_or_default();
            let mut content = original.clone();
            for (index, edit) in edits.iter().enumerate() {
                if edit.old_string.is_empty() {
                    warnings.push(format!("Edit {} did not include old_string.", index + 1));
                    continue;
                }
                if !content.contains(&edit.old_string) {
                    warnings.push(format!(
                        "Edit {} old_string was not found in the current content.",
                        index + 1
                    ));
                    continue;
                }
                content = apply_edit(
                    &content,
                    &edit.old_string,
                    &edit.new_string,
                    edit.replace_all,
                );
            }
            (content, format!("MultiEdit ({} edits)", edits.len()))
        }
        _ => unreachable!(),
    };

    Ok(Some(AgentEditPreview {
        tool_name: tool.to_string(),
        operation,
        file_path: Some(file_path.to_string()),
        hunks: build_hunks(Some(file_path), &original, &modified),
        warnings,
    }))
}

#[tauri::command]
pub async fn build_agent_edit_preview(
    worktree_path: String,
    tool_name: String,
    input: serde_json::Value,
) -> Result<Option<AgentEditPreview>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        build_agent_edit_preview_inner(&worktree_path, &tool_name, &input)
    })
    .await
    .map_err(|e| format!("Failed to build edit preview: {e}"))?
}

fn build_agent_edited_tool_input_inner(
    tool_name: &str,
    input: &serde_json::Value,
    edited_content: &str,
) -> Result<serde_json::Value, String> {
    let mut object = input
        .as_object()
        .cloned()
        .ok_or_else(|| "Tool input must be an object".to_string())?;
    match tool_name.trim() {
        "Edit" => {
            object.insert(
                "new_string".to_string(),
                serde_json::Value::String(edited_content.to_string()),
            );
        }
        "Write" => {
            object.insert(
                "content".to_string(),
                serde_json::Value::String(edited_content.to_string()),
            );
        }
        other => {
            return Err(format!(
                "Direct content editing is not supported for {other}"
            ))
        }
    }
    Ok(serde_json::Value::Object(object))
}

fn build_agent_edited_multi_edit_tool_input_inner(
    input: &serde_json::Value,
    edit_index: usize,
    edited_content: &str,
) -> Result<serde_json::Value, String> {
    let mut object = input
        .as_object()
        .cloned()
        .ok_or_else(|| "Tool input must be an object".to_string())?;
    let edits = object
        .get_mut("edits")
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| "MultiEdit input must include an edits array".to_string())?;
    let edit = edits
        .get_mut(edit_index)
        .and_then(|value| value.as_object_mut())
        .ok_or_else(|| format!("MultiEdit edit index {edit_index} was not found"))?;
    edit.insert(
        "new_string".to_string(),
        serde_json::Value::String(edited_content.to_string()),
    );
    Ok(serde_json::Value::Object(object))
}

fn build_agent_edited_multi_edit_tool_input_all_inner(
    input: &serde_json::Value,
    edited_contents: &[String],
) -> Result<serde_json::Value, String> {
    let mut object = input
        .as_object()
        .cloned()
        .ok_or_else(|| "Tool input must be an object".to_string())?;
    let edits = object
        .get_mut("edits")
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| "MultiEdit input must include an edits array".to_string())?;
    if edits.len() != edited_contents.len() {
        return Err(format!(
            "MultiEdit edited content count {} did not match edit count {}",
            edited_contents.len(),
            edits.len()
        ));
    }
    for (index, edited_content) in edited_contents.iter().enumerate() {
        let edit = edits
            .get_mut(index)
            .and_then(|value| value.as_object_mut())
            .ok_or_else(|| format!("MultiEdit edit index {index} was not an object"))?;
        edit.insert(
            "new_string".to_string(),
            serde_json::Value::String(edited_content.to_string()),
        );
    }
    Ok(serde_json::Value::Object(object))
}

#[tauri::command]
pub fn build_agent_edited_tool_input(
    tool_name: String,
    input: serde_json::Value,
    edited_content: String,
) -> Result<serde_json::Value, String> {
    build_agent_edited_tool_input_inner(&tool_name, &input, &edited_content)
}

#[tauri::command]
pub fn build_agent_edited_multi_edit_tool_input(
    input: serde_json::Value,
    edit_index: usize,
    edited_content: String,
) -> Result<serde_json::Value, String> {
    build_agent_edited_multi_edit_tool_input_inner(&input, edit_index, &edited_content)
}

#[tauri::command]
pub fn build_agent_edited_multi_edit_tool_input_all(
    input: serde_json::Value,
    edited_contents: Vec<String>,
) -> Result<serde_json::Value, String> {
    build_agent_edited_multi_edit_tool_input_all_inner(&input, &edited_contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_agent_edit_preview_returns_edit_diff() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("main.rs"), "fn main() {\n    old();\n}\n").unwrap();

        let preview = build_agent_edit_preview_inner(
            temp.path().to_str().unwrap(),
            "Edit",
            &serde_json::json!({
                "file_path": "main.rs",
                "old_string": "old();",
                "new_string": "new();"
            }),
        )
        .unwrap()
        .unwrap();

        assert_eq!(preview.operation, "Edit first match");
        assert!(preview.hunks[0]
            .lines
            .iter()
            .any(|line| line.kind == AgentEditPreviewLineKind::Removed
                && line.content == "    old();"));
        assert!(preview.hunks[0].lines.iter().any(|line| line.kind
            == AgentEditPreviewLineKind::Added
            && line.content == "    new();"));
    }

    #[test]
    fn build_agent_edited_tool_input_updates_edit_new_string() {
        let input = serde_json::json!({
            "file_path": "src/main.rs",
            "old_string": "old",
            "new_string": "new"
        });

        let result = build_agent_edited_tool_input_inner("Edit", &input, "edited").unwrap();

        assert_eq!(result["new_string"], "edited");
        assert_eq!(result["old_string"], "old");
    }

    #[test]
    fn build_agent_edited_tool_input_updates_write_content() {
        let input = serde_json::json!({
            "file_path": "src/main.rs",
            "content": "old"
        });

        let result = build_agent_edited_tool_input_inner("Write", &input, "edited").unwrap();

        assert_eq!(result["content"], "edited");
    }

    #[test]
    fn build_agent_edited_multi_edit_tool_input_updates_target_operation() {
        let input = serde_json::json!({
            "file_path": "src/main.rs",
            "edits": [
                { "old_string": "one", "new_string": "two" },
                { "old_string": "three", "new_string": "four", "replace_all": true }
            ]
        });

        let result = build_agent_edited_multi_edit_tool_input_inner(&input, 1, "updated").unwrap();

        assert_eq!(result["edits"][0]["new_string"], "two");
        assert_eq!(result["edits"][1]["new_string"], "updated");
        assert_eq!(result["edits"][1]["old_string"], "three");
        assert_eq!(result["edits"][1]["replace_all"], true);
    }

    #[test]
    fn build_agent_edited_multi_edit_tool_input_rejects_missing_index() {
        let input = serde_json::json!({
            "file_path": "src/main.rs",
            "edits": []
        });

        let err = build_agent_edited_multi_edit_tool_input_inner(&input, 0, "updated").unwrap_err();

        assert!(err.contains("index 0"));
    }

    #[test]
    fn build_agent_edited_multi_edit_tool_input_all_updates_all_operations() {
        let input = serde_json::json!({
            "file_path": "src/main.rs",
            "edits": [
                {"old_string": "one", "new_string": "two"},
                {"old_string": "three", "new_string": "four", "replace_all": true}
            ]
        });

        let result = build_agent_edited_multi_edit_tool_input_all_inner(
            &input,
            &["edited one".to_string(), "edited three".to_string()],
        )
        .unwrap();

        assert_eq!(result["edits"][0]["new_string"], "edited one");
        assert_eq!(result["edits"][1]["new_string"], "edited three");
        assert_eq!(result["edits"][1]["replace_all"], true);
    }

    #[test]
    fn build_agent_edited_multi_edit_tool_input_all_rejects_count_mismatch() {
        let input = serde_json::json!({
            "file_path": "src/main.rs",
            "edits": [
                {"old_string": "one", "new_string": "two"}
            ]
        });

        let err = build_agent_edited_multi_edit_tool_input_all_inner(
            &input,
            &["one".to_string(), "two".to_string()],
        )
        .unwrap_err();

        assert!(err.contains("did not match edit count"));
    }

    #[test]
    fn build_agent_edit_preview_rejects_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let err = build_agent_edit_preview_inner(
            temp.path().to_str().unwrap(),
            "Write",
            &serde_json::json!({
                "file_path": "../outside.txt",
                "content": "secret"
            }),
        )
        .unwrap_err();

        assert!(err.contains("traversal"));
    }

    #[test]
    fn build_agent_edit_preview_handles_write_new_file() {
        let temp = tempfile::tempdir().unwrap();
        let preview = build_agent_edit_preview_inner(
            temp.path().to_str().unwrap(),
            "Write",
            &serde_json::json!({
                "file_path": "new.txt",
                "content": "hello\n"
            }),
        )
        .unwrap()
        .unwrap();

        assert_eq!(preview.operation, "Write file");
        assert!(preview
            .warnings
            .iter()
            .any(|warning| warning.contains("new file")));
        assert!(preview.hunks[0]
            .lines
            .iter()
            .any(|line| line.kind == AgentEditPreviewLineKind::Added && line.content == "hello"));
    }

    #[test]
    fn build_agent_edit_preview_ignores_non_edit_tools() {
        let temp = tempfile::tempdir().unwrap();
        let preview = build_agent_edit_preview_inner(
            temp.path().to_str().unwrap(),
            "Read",
            &serde_json::json!({"file_path": "main.rs"}),
        )
        .unwrap();

        assert_eq!(preview, None);
    }
}
