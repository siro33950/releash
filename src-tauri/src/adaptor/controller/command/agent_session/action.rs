use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_skills_list_request, CodexAppServerProcess,
};
use crate::infrastructure::agent_session::runtime::SkillEntry;

#[tauri::command]
pub async fn scan_agent_skills(
    cwd: String,
    backend_id: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SkillEntry>, String> {
    crate::infrastructure::agent_session::runtime::scan_agent_skills(cwd, backend_id, query, limit)
        .await
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

#[tauri::command]
pub async fn read_codex_skill_catalog(
    app: tauri::AppHandle,
    cwd: String,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SkillEntry>, String> {
    let local_fallback = || {
        crate::infrastructure::agent_session::runtime::scan_agent_skills(
            cwd.clone(),
            Some(crate::infrastructure::agent_session::runtime::CODEX_BACKEND_ID.to_string()),
            query.clone(),
            limit,
        )
    };
    let cli_path = configured_cli_path(&app).unwrap_or_else(|| "codex".to_string());
    let mut process = match CodexAppServerProcess::spawn(&cli_path) {
        Ok(process) => process,
        Err(_) => return local_fallback().await,
    };
    let result: Result<Vec<SkillEntry>, String> = async {
        process.initialize(env!("CARGO_PKG_VERSION")).await?;
        let id = process.next_request_id();
        process
            .send(&build_skills_list_request(id, &cwd, false))
            .await?;
        let response = process.read_response_result(id).await?;
        Ok(
            crate::infrastructure::agent_session::runtime::filter_agent_skills_for_query(
                parse_codex_skill_catalog(&response),
                query.as_deref(),
                limit,
            ),
        )
    }
    .await;
    process.shutdown().await;
    match result {
        Ok(skills) => Ok(skills),
        Err(_) => local_fallback().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
