use super::shared::{CLAUDE_BACKEND_ID, CODEX_BACKEND_ID};
use crate::infrastructure::agent_session::runtime::ImageAttachment;
use crate::usecase::agent_session::session::validate_image_bytes;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct SlashCommandEntry {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "argumentHint",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub argument_hint: Option<String>,
}

pub(super) fn normalize_supported_command_name(raw: &str) -> String {
    raw.trim().trim_start_matches('/').to_string()
}

pub(super) fn supported_commands_from_bridge_message(
    msg: &serde_json::Value,
) -> Vec<SlashCommandEntry> {
    let Some(commands) = msg.get("commands").and_then(|value| value.as_array()) else {
        return Vec::new();
    };

    commands
        .iter()
        .filter_map(|command| {
            let obj = command.as_object()?;
            let name = normalize_supported_command_name(obj.get("name")?.as_str()?);
            if name.is_empty() {
                return None;
            }
            let description = obj
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let argument_hint = obj
                .get("argumentHint")
                .or_else(|| obj.get("argument_hint"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            Some(SlashCommandEntry {
                name,
                description,
                argument_hint,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub scope: String,
}

/// Parse SKILL.md frontmatter (delimited by `---`) and extract `name` / `description` fields.
pub(super) fn parse_skill_frontmatter(content: &str) -> Option<(String, String)> {
    let mut lines = content.lines();
    // First line must be `---`
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(val) = trimmed.strip_prefix("name:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = trimmed.strip_prefix("description:") {
            description = Some(val.trim().to_string());
        }
    }
    Some((name.unwrap_or_default(), description.unwrap_or_default()))
}

pub(super) fn normalized_scanner_backend_id(backend_id: Option<&str>) -> &'static str {
    match backend_id {
        Some(id) if id == CODEX_BACKEND_ID => CODEX_BACKEND_ID,
        _ => CLAUDE_BACKEND_ID,
    }
}

pub(super) fn agent_skill_dirs_for_backend(
    cwd: &Path,
    backend_id: Option<&str>,
    home: Option<PathBuf>,
) -> Vec<(PathBuf, &'static str)> {
    let mut dirs = Vec::new();
    match normalized_scanner_backend_id(backend_id) {
        CODEX_BACKEND_ID => {
            if let Some(home) = home {
                dirs.push((home.join(".agents").join("skills"), "personal"));
            }
            dirs.push((cwd.join(".agents").join("skills"), "project"));
        }
        _ => {
            if let Some(home) = home {
                dirs.push((home.join(".claude").join("skills"), "personal"));
            }
            dirs.push((cwd.join(".claude").join("skills"), "project"));
        }
    }
    dirs
}

pub(super) fn scan_agent_skills_inner(
    cwd: &Path,
    backend_id: Option<&str>,
    home: Option<PathBuf>,
) -> Vec<SkillEntry> {
    let mut skills = Vec::new();
    for (skills_dir, scope) in agent_skill_dirs_for_backend(cwd, backend_id, home) {
        if let Ok(entries) = std::fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                if !skill_md.is_file() {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    if let Some((name, description)) = parse_skill_frontmatter(&content) {
                        if !name.is_empty() {
                            skills.push(SkillEntry {
                                name,
                                description,
                                scope: scope.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    skills
}

pub(crate) fn filter_agent_skills_for_query(
    skills: Vec<SkillEntry>,
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<SkillEntry> {
    let needle = query.unwrap_or_default().trim().to_lowercase();
    let max_results = limit.unwrap_or(usize::MAX);
    skills
        .into_iter()
        .filter(|skill| {
            needle.is_empty()
                || skill.name.to_lowercase().contains(&needle)
                || skill.description.to_lowercase().contains(&needle)
        })
        .take(max_results)
        .collect()
}

pub async fn scan_agent_skills(
    cwd: String,
    backend_id: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SkillEntry>, String> {
    let home = dirs::home_dir();
    Ok(filter_agent_skills_for_query(
        scan_agent_skills_inner(&PathBuf::from(cwd), backend_id.as_deref(), home),
        query.as_deref(),
        limit,
    ))
}

// --- Image attachment support ---

/// Validate and encode an image from raw bytes.
/// Returns base64-encoded data and detected MIME type, or an error for unsupported formats.
pub(super) fn validate_and_encode_image(bytes: &[u8]) -> Result<ImageAttachment, String> {
    let media_type = validate_image_bytes(bytes)?;

    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);

    Ok(ImageAttachment {
        data,
        media_type: media_type.to_string(),
    })
}

/// Tauri command: Validate image bytes and return base64-encoded image attachment.
/// Called from the frontend after D&D or paste events.
pub fn prepare_image_attachment(data: Vec<u8>) -> Result<ImageAttachment, String> {
    if data.is_empty() {
        return Err("Empty image data".to_string());
    }
    validate_and_encode_image(&data)
}

/// Tauri command: Read image files from paths and return base64-encoded attachments.
/// Called from the frontend when files are dropped via native drag-and-drop.
/// Non-image files are silently skipped.
pub async fn prepare_image_attachments_from_paths(
    paths: Vec<String>,
) -> Result<Vec<ImageAttachment>, String> {
    let mut attachments = Vec::new();
    for path in &paths {
        let data = tokio::fs::read(path)
            .await
            .map_err(|e| format!("Failed to read {}: {}", path, e))?;
        if data.is_empty() {
            continue;
        }
        if let Ok(attachment) = validate_and_encode_image(&data) {
            attachments.push(attachment);
        }
    }
    Ok(attachments)
}
#[cfg(test)]
mod moved_tests {

    use super::super::shared::*;
    use super::super::skills::*;

    use crate::usecase::agent_session::session::image_attachment::detect_image_mime;

    #[test]
    fn parse_skill_frontmatter_valid() {
        let content = "---\nname: review\ndescription: Code review tool\n---\nBody here";
        let (name, desc) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "review");
        assert_eq!(desc, "Code review tool");
    }

    #[test]
    fn parse_skill_frontmatter_missing_fields() {
        let content = "---\ntitle: something\n---\n";
        let (name, desc) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "");
        assert_eq!(desc, "");
    }

    #[test]
    fn parse_skill_frontmatter_no_opening_delimiter() {
        let content = "name: review\n---\n";
        assert!(parse_skill_frontmatter(content).is_none());
    }

    #[test]
    fn parse_skill_frontmatter_empty_content() {
        assert!(parse_skill_frontmatter("").is_none());
    }

    #[test]
    fn scan_agent_skills_switches_directories_by_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        let claude_skill = cwd.join(".claude").join("skills").join("claude-review");
        std::fs::create_dir_all(&claude_skill).unwrap();
        std::fs::write(
            claude_skill.join("SKILL.md"),
            "---\nname: claude-review\ndescription: Claude review\n---\nBody",
        )
        .unwrap();
        let codex_skill = cwd.join(".agents").join("skills").join("codex-review");
        std::fs::create_dir_all(&codex_skill).unwrap();
        std::fs::write(
            codex_skill.join("SKILL.md"),
            "---\nname: codex-review\ndescription: Codex review\n---\nBody",
        )
        .unwrap();

        let claude = scan_agent_skills_inner(&cwd, Some(CLAUDE_BACKEND_ID), Some(home.clone()));
        let codex = scan_agent_skills_inner(&cwd, Some(CODEX_BACKEND_ID), Some(home));

        assert!(claude.iter().any(|skill| skill.name == "claude-review"));
        assert!(!claude.iter().any(|skill| skill.name == "codex-review"));
        let codex_skill = codex
            .iter()
            .find(|skill| skill.name == "codex-review")
            .expect("Codex project skill should be included");
        assert_eq!(codex_skill.scope, "project");
        assert!(!codex.iter().any(|skill| skill.name == "claude-review"));
    }

    #[test]
    fn scan_agent_skills_preserves_duplicate_codex_skill_names_across_scopes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        let personal_skill = home.join(".agents").join("skills").join("shared-review");
        std::fs::create_dir_all(&personal_skill).unwrap();
        std::fs::write(
            personal_skill.join("SKILL.md"),
            "---\nname: shared-review\ndescription: Personal review\n---\nBody",
        )
        .unwrap();
        let repo_skill = cwd.join(".agents").join("skills").join("shared-review");
        std::fs::create_dir_all(&repo_skill).unwrap();
        std::fs::write(
            repo_skill.join("SKILL.md"),
            "---\nname: shared-review\ndescription: Repo review\n---\nBody",
        )
        .unwrap();

        let codex = scan_agent_skills_inner(&cwd, Some(CODEX_BACKEND_ID), Some(home));

        let matches = codex
            .iter()
            .filter(|skill| skill.name == "shared-review")
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].description, "Personal review");
        assert_eq!(matches[0].scope, "personal");
        assert_eq!(matches[1].description, "Repo review");
        assert_eq!(matches[1].scope, "project");
    }

    #[tokio::test]
    async fn scan_agent_skills_returns_project_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join(".claude").join("skills").join("reviewer");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: reviewer\ndescription: Review focused changes\n---\nBody",
        )
        .unwrap();

        let result = scan_agent_skills(tmp.path().to_string_lossy().to_string(), None, None, None)
            .await
            .unwrap();

        let skill = result.iter().find(|skill| skill.name == "reviewer");
        assert!(skill.is_some(), "project skill should be included");
        let skill = skill.unwrap();
        assert_eq!(skill.description, "Review focused changes");
        assert_eq!(skill.scope, "project");
    }

    #[test]
    fn scan_agent_skills_filters_by_query_and_limit_in_rust() {
        let skills = vec![
            SkillEntry {
                name: "review".to_string(),
                description: "Review code changes".to_string(),
                scope: "project".to_string(),
            },
            SkillEntry {
                name: "docs".to_string(),
                description: "Write documentation".to_string(),
                scope: "personal".to_string(),
            },
            SkillEntry {
                name: "diagram".to_string(),
                description: "Document architecture diagrams".to_string(),
                scope: "project".to_string(),
            },
        ];

        let result = filter_agent_skills_for_query(skills.clone(), Some("doc"), Some(1));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "docs");

        let result = filter_agent_skills_for_query(skills, Some("review"), Some(20));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "review");
    }

    #[test]
    fn supported_commands_from_bridge_message_normalizes_sdk_commands() {
        let msg = serde_json::json!({
            "type": "supported_commands",
            "commands": [
                {
                    "name": "/compact",
                    "description": "Compact context",
                    "argumentHint": "[instructions]"
                },
                {
                    "name": "status",
                    "description": "Show status",
                    "argument_hint": ""
                },
                {
                    "name": "   ",
                    "description": "ignored"
                }
            ]
        });

        let commands = supported_commands_from_bridge_message(&msg);
        assert_eq!(
            commands,
            vec![
                SlashCommandEntry {
                    name: "compact".to_string(),
                    description: "Compact context".to_string(),
                    argument_hint: Some("[instructions]".to_string()),
                },
                SlashCommandEntry {
                    name: "status".to_string(),
                    description: "Show status".to_string(),
                    argument_hint: None,
                },
            ]
        );
    }

    // --- Image attachment tests ---

    #[test]
    fn detect_image_mime_jpeg() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_image_mime(&bytes), Some("image/jpeg"));
    }

    #[test]
    fn detect_image_mime_png() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_image_mime(&bytes), Some("image/png"));
    }

    #[test]
    fn detect_image_mime_gif() {
        let bytes = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61];
        assert_eq!(detect_image_mime(&bytes), Some("image/gif"));
    }

    #[test]
    fn detect_image_mime_webp() {
        let bytes = [
            0x52, 0x49, 0x46, 0x46, // RIFF
            0x00, 0x00, 0x00, 0x00, // size
            0x57, 0x45, 0x42, 0x50, // WEBP
        ];
        assert_eq!(detect_image_mime(&bytes), Some("image/webp"));
    }

    #[test]
    fn detect_image_mime_unknown() {
        let bytes = [0x00, 0x01, 0x02, 0x03];
        assert_eq!(detect_image_mime(&bytes), None);
    }

    #[test]
    fn detect_image_mime_too_short() {
        let bytes = [0xFF, 0xD8];
        assert_eq!(detect_image_mime(&bytes), None);
    }

    #[test]
    fn validate_and_encode_image_jpeg() {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        bytes.extend_from_slice(&[0x00; 100]); // pad
        let result = validate_and_encode_image(&bytes).unwrap();
        assert_eq!(result.media_type, "image/jpeg");
        assert!(!result.data.is_empty());
    }

    #[test]
    fn validate_and_encode_image_png() {
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47];
        bytes.extend_from_slice(&[0x00; 100]);
        let result = validate_and_encode_image(&bytes).unwrap();
        assert_eq!(result.media_type, "image/png");
    }

    #[test]
    fn validate_and_encode_image_rejects_unknown() {
        let bytes = vec![0x00, 0x01, 0x02, 0x03, 0x04];
        let result = validate_and_encode_image(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported"));
    }

    #[test]
    fn prepare_image_attachment_empty_data() {
        let result = prepare_image_attachment(vec![]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty"));
    }

    #[test]
    fn prepare_image_attachment_valid_png() {
        let mut bytes = vec![0x89, 0x50, 0x4E, 0x47];
        bytes.extend_from_slice(&[0x00; 100]);
        let result = prepare_image_attachment(bytes).unwrap();
        assert_eq!(result.media_type, "image/png");
    }

    #[test]
    fn prepare_image_attachment_rejects_text_file() {
        let bytes = b"Hello, world!".to_vec();
        let result = prepare_image_attachment(bytes);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn prepare_image_attachments_from_paths_valid_png() {
        let dir = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("test.png");
        let mut png_bytes = vec![0x89, 0x50, 0x4E, 0x47];
        png_bytes.extend_from_slice(&[0x00; 100]);
        tokio::fs::write(&png_path, &png_bytes).await.unwrap();

        let result =
            prepare_image_attachments_from_paths(vec![png_path.to_string_lossy().to_string()])
                .await
                .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].media_type, "image/png");
    }

    #[tokio::test]
    async fn prepare_image_attachments_from_paths_skips_non_image() {
        let dir = tempfile::tempdir().unwrap();
        let txt_path = dir.path().join("readme.txt");
        tokio::fs::write(&txt_path, b"Hello, world!").await.unwrap();

        let result =
            prepare_image_attachments_from_paths(vec![txt_path.to_string_lossy().to_string()])
                .await
                .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn prepare_image_attachments_from_paths_skips_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let empty_path = dir.path().join("empty.png");
        tokio::fs::write(&empty_path, b"").await.unwrap();

        let result =
            prepare_image_attachments_from_paths(vec![empty_path.to_string_lossy().to_string()])
                .await
                .unwrap();
        assert!(result.is_empty());
    }
}
