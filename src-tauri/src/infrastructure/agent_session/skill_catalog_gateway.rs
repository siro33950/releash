use std::path::Path;

use crate::domain::agent_session::{services::filter_agent_skills_for_query, SkillEntry};
use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_skills_list_request, CodexAppServerProcess,
};
use crate::infrastructure::agent_session::runtime::scan_agent_skills_inner;
use crate::usecase::agent_session::skill_catalog::CodexSkillCatalogGateway;

pub(crate) struct TauriCodexSkillCatalogGateway<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriCodexSkillCatalogGateway<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl<R: tauri::Runtime + 'static> CodexSkillCatalogGateway for TauriCodexSkillCatalogGateway<R> {
    async fn list_app_server_skills(
        &self,
        cwd: &str,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String> {
        let cli_path = configured_cli_path(&self.app).unwrap_or_else(|| "codex".to_string());
        let mut process = CodexAppServerProcess::spawn(&self.app, &cli_path).await?;
        let result = async {
            process.initialize(env!("CARGO_PKG_VERSION")).await?;
            let id = process.next_request_id();
            process
                .send(&build_skills_list_request(id, cwd, false))
                .await?;
            let response = process.read_response_result(id).await?;
            Ok(filter_agent_skills_for_query(
                parse_codex_skill_catalog(&response),
                query,
                limit,
            ))
        }
        .await;
        process.shutdown().await;
        result
    }

    async fn scan_local_skills(
        &self,
        cwd: &str,
        backend_id: Option<&str>,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String> {
        Ok(filter_agent_skills_for_query(
            scan_agent_skills_inner(Path::new(cwd), backend_id, dirs::home_dir()),
            query,
            limit,
        ))
    }
}

fn codex_skill_scope(value: &str) -> String {
    match value {
        "user" => "personal".to_string(),
        "repo" => "project".to_string(),
        "system" | "admin" => value.to_string(),
        _ => "project".to_string(),
    }
}

fn parse_codex_skill_catalog(value: &serde_json::Value) -> Vec<SkillEntry> {
    let mut skills = Vec::new();
    let Some(entries) = value.get("data").and_then(serde_json::Value::as_array) else {
        return skills;
    };
    for entry in entries {
        let Some(items) = entry.get("skills").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for skill in items {
            if !skill
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
            {
                continue;
            }
            let Some(name) = skill
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let description = skill
                .get("interface")
                .and_then(|interface| interface.get("shortDescription"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    skill
                        .get("shortDescription")
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| skill.get("description").and_then(serde_json::Value::as_str))
                .unwrap_or("")
                .trim()
                .to_string();
            let scope = skill
                .get("scope")
                .and_then(serde_json::Value::as_str)
                .map(codex_skill_scope)
                .unwrap_or_else(|| "project".to_string());
            skills.push(SkillEntry {
                name: name.to_string(),
                description,
                scope,
            });
        }
    }
    skills
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::agent_session::runtime::{CLAUDE_BACKEND_ID, CODEX_BACKEND_ID};

    #[test]
    fn parses_codex_skill_catalog_enabled_scopes_and_short_description() {
        let response = serde_json::json!({
            "data": [
                {
                    "cwd": "/repo",
                    "errors": [],
                    "skills": [
                        {
                            "name": "review",
                            "description": "Long review description",
                            "enabled": true,
                            "path": "/repo/.agents/skills/review/SKILL.md",
                            "scope": "repo",
                            "shortDescription": "Review changes",
                            "interface": { "shortDescription": "Runtime review" }
                        },
                        {
                            "name": "draft",
                            "description": "Draft docs",
                            "enabled": true,
                            "path": "/home/.agents/skills/draft/SKILL.md",
                            "scope": "user",
                            "shortDescription": null,
                            "interface": null
                        },
                        {
                            "name": "disabled",
                            "description": "Disabled",
                            "enabled": false,
                            "path": "/repo/.agents/skills/disabled/SKILL.md",
                            "scope": "repo"
                        },
                        {
                            "name": "builtin",
                            "description": "Builtin",
                            "enabled": true,
                            "path": "/app/skills/builtin/SKILL.md",
                            "scope": "system"
                        }
                    ]
                }
            ]
        });

        let skills = parse_codex_skill_catalog(&response);

        assert_eq!(
            skills,
            vec![
                SkillEntry {
                    name: "review".to_string(),
                    description: "Runtime review".to_string(),
                    scope: "project".to_string(),
                },
                SkillEntry {
                    name: "draft".to_string(),
                    description: "Draft docs".to_string(),
                    scope: "personal".to_string(),
                },
                SkillEntry {
                    name: "builtin".to_string(),
                    description: "Builtin".to_string(),
                    scope: "system".to_string(),
                },
            ]
        );
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
    async fn scan_local_skills_returns_filtered_project_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join(".claude").join("skills").join("reviewer");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: unique-reviewer-xyz\ndescription: Unique focused changes\n---\nBody",
        )
        .unwrap();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let gateway = TauriCodexSkillCatalogGateway::new(app.handle().clone());

        let result = gateway
            .scan_local_skills(
                tmp.path().to_string_lossy().as_ref(),
                None,
                Some("unique-reviewer-xyz"),
                Some(10),
            )
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "unique-reviewer-xyz");
        assert_eq!(result[0].description, "Unique focused changes");
        assert_eq!(result[0].scope, "project");
    }
}
