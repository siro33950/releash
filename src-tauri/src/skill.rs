use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMcpConfig {
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_agent")]
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    pub command: String,
    pub prompt_template: String,
    #[serde(default = "default_timeout")]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub mcp_config: Option<SkillMcpConfig>,
}

fn default_agent() -> String {
    "claude".to_string()
}

fn default_timeout() -> Option<u64> {
    Some(300)
}

const DEFAULT_REVIEW_PROMPT: &str = include_str!("../resources/prompts/review.txt");

fn default_skills() -> Vec<SkillDefinition> {
    vec![SkillDefinition {
        name: "code-review".to_string(),
        description: Some("AI code review using MCP tools".to_string()),
        agent: "claude".to_string(),
        model: None,
        command: "claude --print \"{prompt}\"".to_string(),
        prompt_template: DEFAULT_REVIEW_PROMPT.to_string(),
        timeout: Some(300),
        mcp_config: Some(SkillMcpConfig {
            tools: vec![
                "worktrees_list".to_string(),
                "post_review_comment".to_string(),
                "get_review_comments".to_string(),
                "resolve_comment".to_string(),
            ],
        }),
    }]
}

#[derive(Debug, Deserialize)]
struct SkillsFile {
    #[serde(default)]
    skills: Vec<SkillDefinition>,
}

pub fn load_skills(repo_path: &str) -> Vec<SkillDefinition> {
    let mut skills_map: HashMap<String, SkillDefinition> = HashMap::new();

    // Load built-in defaults
    for skill in default_skills() {
        skills_map.insert(skill.name.clone(), skill);
    }

    // Try to load user overrides from .releash/skills.toml
    let skills_path = Path::new(repo_path).join(".releash").join("skills.toml");
    if skills_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&skills_path) {
            if let Ok(file) = toml::from_str::<SkillsFile>(&content) {
                for skill in file.skills {
                    // Same name overrides, new name adds
                    skills_map.insert(skill.name.clone(), skill);
                }
            } else {
                log::warn!("Failed to parse {:?}", skills_path);
            }
        }
    }

    let mut result: Vec<SkillDefinition> = skills_map.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_skills(repo_path: String) -> Vec<SkillDefinition> {
    load_skills(&repo_path)
}

fn register_review_skill_to(root: &Path, agent: &str, skill_name: &str) -> Result<String, String> {
    let prompt = DEFAULT_REVIEW_PROMPT;

    let (dir, file_name, content) = match agent {
        "claude" => {
            let dir = root.join(".claude").join("skills").join(skill_name);
            let content = format!(
                "---\nname: {skill_name}\ndescription: AI code review using Releash MCP\n---\n\n{prompt}",
            );
            (dir, "SKILL.md".to_string(), content)
        }
        "codex" => {
            let dir = root.join(".agents").join("skills").join(skill_name);
            let content = format!(
                "---\nname: {skill_name}\ndescription: AI code review using Releash MCP\n---\n\n{prompt}",
            );
            (dir, "SKILL.md".to_string(), content)
        }
        "gemini" => {
            let dir = root.join(".gemini").join("commands");
            let content = format!(
                "[command]\nname = \"{skill_name}\"\ndescription = \"AI code review using Releash MCP\"\n\n[prompt]\ntext = \"\"\"\n{prompt}\n\"\"\"\n",
            );
            (dir, format!("{skill_name}.toml"), content)
        }
        "cursor" => {
            let dir = root.join(".cursor").join("commands");
            let content =
                format!("# {skill_name}\n\nAI code review using Releash MCP\n\n{prompt}",);
            (dir, format!("{skill_name}.md"), content)
        }
        other => {
            return Err(format!("Unsupported agent for skill registration: {other}"));
        }
    };

    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create directory: {e}"))?;
    let file_path = dir.join(&file_name);
    std::fs::write(&file_path, content).map_err(|e| format!("Failed to write skill file: {e}"))?;

    Ok(file_path.to_string_lossy().to_string())
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Could not determine home directory".to_string())
}

#[tauri::command]
pub fn register_review_skill(agent: String, skill_name: String) -> Result<String, String> {
    let root = home_dir()?;
    register_review_skill_to(&root, &agent, &skill_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_skills_contains_code_review() {
        let skills = default_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "code-review");
        assert!(!skills[0].prompt_template.is_empty());
    }

    #[test]
    fn load_skills_returns_defaults_when_no_file() {
        let dir = TempDir::new().unwrap();
        let skills = load_skills(dir.path().to_str().unwrap());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "code-review");
    }

    #[test]
    fn load_skills_merges_user_overrides() {
        let dir = TempDir::new().unwrap();
        let releash_dir = dir.path().join(".releash");
        std::fs::create_dir_all(&releash_dir).unwrap();

        let toml_content = r#"
[[skills]]
name = "code-review"
description = "Custom review"
command = "custom-cmd"
prompt_template = "custom prompt"

[[skills]]
name = "security-check"
description = "Security audit"
command = "security-cmd"
prompt_template = "security prompt"
"#;
        std::fs::write(releash_dir.join("skills.toml"), toml_content).unwrap();

        let skills = load_skills(dir.path().to_str().unwrap());
        assert_eq!(skills.len(), 2);

        let review = skills.iter().find(|s| s.name == "code-review").unwrap();
        assert_eq!(review.description.as_deref(), Some("Custom review"));
        assert_eq!(review.command, "custom-cmd");

        let security = skills.iter().find(|s| s.name == "security-check").unwrap();
        assert_eq!(security.description.as_deref(), Some("Security audit"));
    }

    #[test]
    fn load_skills_ignores_invalid_toml() {
        let dir = TempDir::new().unwrap();
        let releash_dir = dir.path().join(".releash");
        std::fs::create_dir_all(&releash_dir).unwrap();
        std::fs::write(releash_dir.join("skills.toml"), "invalid toml {{{{").unwrap();

        let skills = load_skills(dir.path().to_str().unwrap());
        // Falls back to defaults
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "code-review");
    }

    #[test]
    fn skill_definition_serialization() {
        let skill = &default_skills()[0];
        let json = serde_json::to_string(skill).unwrap();
        assert!(json.contains("\"name\":\"code-review\""));
        let deserialized: SkillDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "code-review");
    }

    #[test]
    fn register_review_skill_claude() {
        let dir = TempDir::new().unwrap();
        let result = register_review_skill_to(dir.path(), "claude", "code-review");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.contains(".claude/skills/code-review/SKILL.md"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("---\nname: code-review"));
        assert!(content.contains("You are a code reviewer"));
    }

    #[test]
    fn register_review_skill_codex() {
        let dir = TempDir::new().unwrap();
        let result = register_review_skill_to(dir.path(), "codex", "my-review");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.contains(".agents/skills/my-review/SKILL.md"));
    }

    #[test]
    fn register_review_skill_gemini() {
        let dir = TempDir::new().unwrap();
        let result = register_review_skill_to(dir.path(), "gemini", "code-review");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.contains(".gemini/commands/code-review.toml"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[command]"));
        assert!(content.contains("name = \"code-review\""));
    }

    #[test]
    fn register_review_skill_cursor() {
        let dir = TempDir::new().unwrap();
        let result = register_review_skill_to(dir.path(), "cursor", "code-review");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.contains(".cursor/commands/code-review.md"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("# code-review"));
    }

    #[test]
    fn register_review_skill_unsupported_agent() {
        let dir = TempDir::new().unwrap();
        let result = register_review_skill_to(dir.path(), "aider", "code-review");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported agent"));
    }
}
