use crate::config::ReleashConfig;
use crate::infrastructure::agent_session::runtime::codex::configured_cli_path;
use crate::infrastructure::agent_session::runtime::codex_app_server::{
    build_skills_list_request, CodexAppServerProcess,
};
use crate::infrastructure::agent_session::runtime::{
    AgentBackendRegistry, BackendInfo, SkillEntry,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const AGENTS_MD_TEMPLATE: &str = r#"# Agent Instructions

## Project Structure
- Describe the main source, test, documentation, and generated-output directories.

## Build, Test, and Lint
- List the exact commands agents should run before handing work back.
- Call out any commands that are slow, flaky, destructive, or require credentials.

## Coding Conventions
- Document formatting, naming, architecture, dependency, and error-handling rules.
- Note where business logic belongs and which layers should stay presentation-only.

## Testing Guidance
- Explain where tests live and which cases need unit, integration, or manual coverage.
- Include important mocks, fixtures, and environment setup notes.

## Review Expectations
- Define what "done" means for this repository.
- Include security, performance, accessibility, and backward-compatibility checks that matter here.

## Do Not
- List files, commands, or workflows agents must avoid unless explicitly requested.
"#;

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

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInitScaffoldResult {
    pub path: String,
    pub created: bool,
    pub content: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentDoctorReport {
    pub title: String,
    pub detail: String,
    pub ok_count: usize,
    pub warning_count: usize,
    pub error_count: usize,
}

fn bool_label(value: bool) -> &'static str {
    if value {
        "enabled"
    } else {
        "disabled"
    }
}

fn present_label(value: &str) -> &'static str {
    if value.trim().is_empty() {
        "missing"
    } else {
        "present"
    }
}

fn file_state(path: PathBuf) -> String {
    match path.try_exists() {
        Ok(true) => format!("present ({})", path.display()),
        Ok(false) => format!("missing ({})", path.display()),
        Err(e) => format!("unknown: {e} ({})", path.display()),
    }
}

fn discover_git_state(worktree_path: &str) -> String {
    match git2::Repository::discover(worktree_path) {
        Ok(repo) => repo
            .workdir()
            .or_else(|| repo.path().parent())
            .map(|path| format!("present ({})", path.display()))
            .unwrap_or_else(|| "present".to_string()),
        Err(e) => format!("not discovered: {e}"),
    }
}

fn format_backend_line(backend: &BackendInfo) -> String {
    let models = backend.available_models.len();
    let first_model = backend
        .available_models
        .first()
        .map(|model| model.value.as_str())
        .unwrap_or("(none)");
    format!(
        "- {} ({}): {}, models: {models}, first: {first_model}",
        backend.name,
        backend.id,
        if backend.available {
            "available"
        } else {
            "unavailable"
        }
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoctorStatus {
    Ok,
    Warn,
    Error,
}

fn push_doctor_check(
    lines: &mut Vec<String>,
    ok_count: &mut usize,
    warning_count: &mut usize,
    error_count: &mut usize,
    status: DoctorStatus,
    label: &str,
    detail: impl Into<String>,
) {
    match status {
        DoctorStatus::Ok => {
            *ok_count += 1;
            lines.push(format!("OK {label}: {}", detail.into()));
        }
        DoctorStatus::Warn => {
            *warning_count += 1;
            lines.push(format!("WARN {label}: {}", detail.into()));
        }
        DoctorStatus::Error => {
            *error_count += 1;
            lines.push(format!("ERROR {label}: {}", detail.into()));
        }
    }
}

fn build_agent_doctor_report_inner(
    config: &ReleashConfig,
    config_path: &Path,
    worktree_path: &str,
    backends: &[BackendInfo],
    default_backend_id: Option<&str>,
) -> AgentDoctorReport {
    let mut lines = Vec::new();
    let mut ok_count = 0;
    let mut warning_count = 0;
    let mut error_count = 0;
    let worktree = Path::new(worktree_path);

    if worktree_path.trim().is_empty() {
        push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            DoctorStatus::Error,
            "Worktree",
            "path is empty",
        );
    } else if worktree.is_dir() {
        push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            DoctorStatus::Ok,
            "Worktree",
            worktree.display().to_string(),
        );
    } else {
        push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            DoctorStatus::Error,
            "Worktree",
            format!("not a directory ({})", worktree.display()),
        );
    }

    match git2::Repository::discover(worktree_path) {
        Ok(repo) => {
            let detail = repo
                .workdir()
                .or_else(|| repo.path().parent())
                .map(|path| format!("present ({})", path.display()))
                .unwrap_or_else(|| "present".to_string());
            push_doctor_check(
                &mut lines,
                &mut ok_count,
                &mut warning_count,
                &mut error_count,
                DoctorStatus::Ok,
                "Git repository",
                detail,
            );
        }
        Err(e) => push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            DoctorStatus::Error,
            "Git repository",
            format!("not discovered: {e}"),
        ),
    }

    let config_status = match config_path.try_exists() {
        Ok(true) => (
            DoctorStatus::Ok,
            format!("present ({})", config_path.display()),
        ),
        Ok(false) => (
            DoctorStatus::Warn,
            format!("missing ({})", config_path.display()),
        ),
        Err(e) => (
            DoctorStatus::Warn,
            format!("unknown: {e} ({})", config_path.display()),
        ),
    };
    push_doctor_check(
        &mut lines,
        &mut ok_count,
        &mut warning_count,
        &mut error_count,
        config_status.0,
        "Releash config",
        config_status.1,
    );

    let agents_md = worktree.join("AGENTS.md");
    let agents_status = match agents_md.try_exists() {
        Ok(true) => (
            DoctorStatus::Ok,
            format!("present ({})", agents_md.display()),
        ),
        Ok(false) => (
            DoctorStatus::Warn,
            format!("missing ({})", agents_md.display()),
        ),
        Err(e) => (
            DoctorStatus::Warn,
            format!("unknown: {e} ({})", agents_md.display()),
        ),
    };
    push_doctor_check(
        &mut lines,
        &mut ok_count,
        &mut warning_count,
        &mut error_count,
        agents_status.0,
        "AGENTS.md",
        agents_status.1,
    );

    let project_codex_config = worktree.join(".codex").join("config.toml");
    let codex_config_status = match project_codex_config.try_exists() {
        Ok(true) => (
            DoctorStatus::Ok,
            format!("present ({})", project_codex_config.display()),
        ),
        Ok(false) => (
            DoctorStatus::Warn,
            format!("missing ({})", project_codex_config.display()),
        ),
        Err(e) => (
            DoctorStatus::Warn,
            format!("unknown: {e} ({})", project_codex_config.display()),
        ),
    };
    push_doctor_check(
        &mut lines,
        &mut ok_count,
        &mut warning_count,
        &mut error_count,
        codex_config_status.0,
        "Project Codex config",
        codex_config_status.1,
    );

    if backends.is_empty() {
        push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            DoctorStatus::Error,
            "Backends",
            "no registered agent backends",
        );
    } else {
        let available = backends.iter().filter(|backend| backend.available).count();
        let status = if available == 0 {
            DoctorStatus::Error
        } else {
            DoctorStatus::Ok
        };
        push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            status,
            "Backends",
            format!("{available}/{} available", backends.len()),
        );
    }

    match default_backend_id {
        Some(default_backend_id) => {
            let maybe_backend = backends
                .iter()
                .find(|backend| backend.id == default_backend_id);
            match maybe_backend {
                Some(backend) if backend.available => push_doctor_check(
                    &mut lines,
                    &mut ok_count,
                    &mut warning_count,
                    &mut error_count,
                    DoctorStatus::Ok,
                    "Default backend",
                    format!("{} ({})", backend.name, backend.id),
                ),
                Some(backend) => push_doctor_check(
                    &mut lines,
                    &mut ok_count,
                    &mut warning_count,
                    &mut error_count,
                    DoctorStatus::Error,
                    "Default backend",
                    format!("{} ({}) is unavailable", backend.name, backend.id),
                ),
                None => push_doctor_check(
                    &mut lines,
                    &mut ok_count,
                    &mut warning_count,
                    &mut error_count,
                    DoctorStatus::Error,
                    "Default backend",
                    format!("{default_backend_id} is not registered"),
                ),
            }
        }
        None => push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            DoctorStatus::Warn,
            "Default backend",
            "not resolved",
        ),
    }

    match config
        .agents
        .codex
        .cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(cli_path) => {
            let cli = Path::new(cli_path);
            let status = if cli.is_absolute() && !cli.exists() {
                DoctorStatus::Warn
            } else {
                DoctorStatus::Ok
            };
            let detail = if cli.is_absolute() && !cli.exists() {
                format!("configured but missing ({cli_path})")
            } else {
                cli_path.to_string()
            };
            push_doctor_check(
                &mut lines,
                &mut ok_count,
                &mut warning_count,
                &mut error_count,
                status,
                "Codex CLI path",
                detail,
            );
        }
        None => push_doctor_check(
            &mut lines,
            &mut ok_count,
            &mut warning_count,
            &mut error_count,
            DoctorStatus::Warn,
            "Codex CLI path",
            "not configured; falling back to PATH lookup",
        ),
    }

    push_doctor_check(
        &mut lines,
        &mut ok_count,
        &mut warning_count,
        &mut error_count,
        DoctorStatus::Ok,
        "Hook endpoint",
        format!(
            "localhost:{} ({})",
            config.server.hook_port,
            present_label(&config.server.token)
        ),
    );
    push_doctor_check(
        &mut lines,
        &mut ok_count,
        &mut warning_count,
        &mut error_count,
        DoctorStatus::Ok,
        "MCP endpoint",
        format!(
            "localhost:{} ({})",
            config.server.mcp_port,
            present_label(&config.server.mcp_token)
        ),
    );

    let title = if error_count > 0 {
        format!("Doctor: {error_count} error(s), {warning_count} warning(s)")
    } else if warning_count > 0 {
        format!("Doctor: {warning_count} warning(s)")
    } else {
        "Doctor: all checks passed".to_string()
    };

    AgentDoctorReport {
        title,
        detail: lines.join("\n"),
        ok_count,
        warning_count,
        error_count,
    }
}

fn build_agent_debug_config_report_inner(
    config: &ReleashConfig,
    config_path: &Path,
    worktree_path: &str,
    backends: &[BackendInfo],
    default_backend_id: Option<&str>,
) -> String {
    let config_path_display = config_path.display();
    let config_file_state = file_state(config_path.to_path_buf());
    let worktree = Path::new(worktree_path);
    let agents_md = file_state(worktree.join("AGENTS.md"));
    let project_codex_config = file_state(worktree.join(".codex").join("config.toml"));
    let project_claude_settings = file_state(worktree.join(".claude").join("settings.json"));
    let git_state = discover_git_state(worktree_path);
    let codex_cli = config
        .agents
        .codex
        .cli_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("codex");
    let default_backend = default_backend_id.unwrap_or("(unresolved)");
    let backend_lines = if backends.is_empty() {
        "- (none)".to_string()
    } else {
        backends
            .iter()
            .map(format_backend_line)
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "Config layers\n- Releash config: {config_file_state}\n- Active config path: {config_path_display}\n- Project Codex config: {project_codex_config}\n- Project Claude settings: {project_claude_settings}\n\nAgent guidance\n- Worktree: {worktree_path}\n- Git repository: {git_state}\n- AGENTS.md: {agents_md}\n\nAgent backend\n- Configured default: {}\n- Resolved default: {default_backend}\n- Codex CLI path: {codex_cli}\n{backend_lines}\n\nRuntime requirements\n- Server bind: {}:{}\n- Hook endpoint: localhost:{} ({})\n- MCP endpoint: localhost:{} ({})\n- TLS: {}\n- Telemetry: {}\n- Workflow auto-approve: {}\n",
        config.agents.default.as_deref().unwrap_or("(not set)"),
        config.server.bind,
        config.server.port,
        config.server.hook_port,
        present_label(&config.server.token),
        config.server.mcp_port,
        present_label(&config.server.mcp_token),
        bool_label(config.server.tls.enabled),
        bool_label(config.telemetry_enabled),
        bool_label(config.workflow.approval_auto_approve),
    )
}

#[tauri::command]
pub fn build_agent_debug_config_report(
    worktree_path: String,
    app_config: tauri::State<'_, Arc<crate::config::AppConfig>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
) -> Result<String, String> {
    let config = app_config.get_config()?;
    let config_path = app_config.config_path();
    let backends = registry.list();
    let default_backend_id = registry.resolve_default_id().ok();
    Ok(build_agent_debug_config_report_inner(
        &config,
        &config_path,
        &worktree_path,
        &backends,
        default_backend_id.as_deref(),
    ))
}

#[tauri::command]
pub fn build_agent_doctor_report(
    worktree_path: String,
    app_config: tauri::State<'_, Arc<crate::config::AppConfig>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
) -> Result<AgentDoctorReport, String> {
    let config = app_config.get_config()?;
    let config_path = app_config.config_path();
    let backends = registry.list();
    let default_backend_id = registry.resolve_default_id().ok();
    Ok(build_agent_doctor_report_inner(
        &config,
        &config_path,
        &worktree_path,
        &backends,
        default_backend_id.as_deref(),
    ))
}

fn create_agents_md_scaffold_inner(worktree_path: &str) -> Result<AgentInitScaffoldResult, String> {
    let root = Path::new(worktree_path);
    if worktree_path.trim().is_empty() {
        return Err("Worktree path is empty".to_string());
    }
    if !root.is_dir() {
        return Err(format!("Worktree path is not a directory: {worktree_path}"));
    }

    let target = root.join("AGENTS.md");
    let target_display = target.to_string_lossy().to_string();
    if target.exists() {
        return Ok(AgentInitScaffoldResult {
            path: target_display,
            created: false,
            content: String::new(),
        });
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| format!("Failed to create AGENTS.md: {e}"))?;
    file.write_all(AGENTS_MD_TEMPLATE.as_bytes())
        .map_err(|e| format!("Failed to write AGENTS.md: {e}"))?;

    Ok(AgentInitScaffoldResult {
        path: target_display,
        created: true,
        content: AGENTS_MD_TEMPLATE.to_string(),
    })
}

#[tauri::command]
pub fn create_agents_md_scaffold(worktree_path: String) -> Result<AgentInitScaffoldResult, String> {
    create_agents_md_scaffold_inner(&worktree_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_agents_md_scaffold_creates_starter_file() {
        let tmp = tempfile::tempdir().unwrap();

        let result = create_agents_md_scaffold_inner(&tmp.path().to_string_lossy()).unwrap();

        assert!(result.created);
        assert!(result.path.ends_with("AGENTS.md"));
        assert!(result.content.contains("Build, Test, and Lint"));
        let written = std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert_eq!(written, result.content);
    }

    #[test]
    fn create_agents_md_scaffold_does_not_overwrite_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("AGENTS.md");
        std::fs::write(&target, "existing instructions").unwrap();

        let result = create_agents_md_scaffold_inner(&tmp.path().to_string_lossy()).unwrap();

        assert!(!result.created);
        assert!(result.content.is_empty());
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            "existing instructions"
        );
    }

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
    fn build_agent_debug_config_report_includes_layers_and_sanitized_requirements() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("releash.toml");
        std::fs::write(&config_path, "config").unwrap();
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(tmp.path().join(".codex").join("config.toml"), "model = 'x'").unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "instructions").unwrap();
        let mut config = ReleashConfig::default();
        config.server.token = "secret-token".to_string();
        config.server.mcp_token = "secret-mcp-token".to_string();
        config.agents.default = Some("codex".to_string());
        config.agents.codex.cli_path = Some("/usr/local/bin/codex".to_string());
        let backends = vec![BackendInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            available: true,
            available_models: vec![crate::infrastructure::agent_session::runtime::ModelInfo {
                value: "gpt-5.5".to_string(),
            }],
        }];

        let report = build_agent_debug_config_report_inner(
            &config,
            &config_path,
            &tmp.path().to_string_lossy(),
            &backends,
            Some("codex"),
        );

        assert!(report.contains("Config layers"));
        assert!(report.contains("AGENTS.md: present"));
        assert!(report.contains("Project Codex config: present"));
        assert!(report.contains("Resolved default: codex"));
        assert!(report.contains("Codex CLI path: /usr/local/bin/codex"));
        assert!(report.contains("Hook endpoint: localhost:19700 (present)"));
        assert!(!report.contains("secret-token"));
        assert!(!report.contains("secret-mcp-token"));
    }

    #[test]
    fn build_agent_doctor_report_passes_for_available_backend_and_repo() {
        let tmp = tempfile::tempdir().unwrap();
        git2::Repository::init(tmp.path()).unwrap();
        let config_path = tmp.path().join("releash.toml");
        std::fs::write(&config_path, "config").unwrap();
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(tmp.path().join(".codex").join("config.toml"), "model = 'x'").unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "instructions").unwrap();
        let mut config = ReleashConfig::default();
        config.agents.default = Some("codex".to_string());
        config.agents.codex.cli_path = Some("codex".to_string());
        let backends = vec![BackendInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            available: true,
            available_models: vec![crate::infrastructure::agent_session::runtime::ModelInfo {
                value: "gpt-5.5".to_string(),
            }],
        }];

        let report = build_agent_doctor_report_inner(
            &config,
            &config_path,
            &tmp.path().to_string_lossy(),
            &backends,
            Some("codex"),
        );

        assert_eq!(report.title, "Doctor: all checks passed");
        assert_eq!(report.error_count, 0);
        assert_eq!(report.warning_count, 0);
        assert!(report.detail.contains("OK Worktree:"));
        assert!(report.detail.contains("OK Git repository: present"));
        assert!(report.detail.contains("OK Default backend: Codex (codex)"));
    }

    #[test]
    fn build_agent_doctor_report_reports_errors_without_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let missing_worktree = tmp.path().join("missing");
        let config_path = tmp.path().join("missing-releash.toml");
        let mut config = ReleashConfig::default();
        config.server.token = "secret-token".to_string();
        config.server.mcp_token = "secret-mcp-token".to_string();
        config.agents.default = Some("codex".to_string());
        let backends = vec![BackendInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            available: false,
            available_models: Vec::new(),
        }];

        let report = build_agent_doctor_report_inner(
            &config,
            &config_path,
            &missing_worktree.to_string_lossy(),
            &backends,
            Some("codex"),
        );

        assert!(report.error_count >= 3);
        assert!(report.title.starts_with("Doctor:"));
        assert!(report.detail.contains("ERROR Worktree: not a directory"));
        assert!(report
            .detail
            .contains("ERROR Default backend: Codex (codex) is unavailable"));
        assert!(report
            .detail
            .contains("Hook endpoint: localhost:19700 (present)"));
        assert!(!report.detail.contains("secret-token"));
        assert!(!report.detail.contains("secret-mcp-token"));
    }
}
