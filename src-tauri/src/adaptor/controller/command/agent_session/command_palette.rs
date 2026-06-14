use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::config::{AgentShortcutSection, AppConfig};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommandPaletteRequest {
    pub has_active_session: bool,
    pub session_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommandPaletteItem {
    pub id: String,
    pub label: String,
    pub shortcut: String,
    pub alternate_shortcut: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentShortcutSetting {
    pub id: String,
    pub label: String,
    pub shortcut: String,
    pub alternate_shortcut: Option<String>,
    pub default_shortcut: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentShortcutUpdate {
    pub id: String,
    pub shortcut: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommandEnabledRequest {
    pub command_id: String,
    pub request: AgentCommandPaletteRequest,
}

#[derive(Debug, Clone, Copy)]
struct AgentShortcutDefinition {
    id: &'static str,
    label: &'static str,
    default_shortcut: &'static str,
    alternate_shortcut: Option<&'static str>,
}

const AGENT_SHORTCUTS: &[AgentShortcutDefinition] = &[
    AgentShortcutDefinition {
        id: "command_menu",
        label: "Command menu",
        default_shortcut: "Cmd K",
        alternate_shortcut: Some("Cmd Shift P"),
    },
    AgentShortcutDefinition {
        id: "new_thread",
        label: "New thread",
        default_shortcut: "Cmd N",
        alternate_shortcut: None,
    },
    AgentShortcutDefinition {
        id: "search_threads",
        label: "Search threads",
        default_shortcut: "Cmd G",
        alternate_shortcut: None,
    },
    AgentShortcutDefinition {
        id: "find_in_thread",
        label: "Find in thread",
        default_shortcut: "Cmd F",
        alternate_shortcut: None,
    },
    AgentShortcutDefinition {
        id: "copy_latest_response",
        label: "Copy latest response",
        default_shortcut: "Ctrl O",
        alternate_shortcut: None,
    },
    AgentShortcutDefinition {
        id: "toggle_raw_scrollback",
        label: "Toggle raw scrollback",
        default_shortcut: "",
        alternate_shortcut: None,
    },
    AgentShortcutDefinition {
        id: "previous_thread",
        label: "Previous thread",
        default_shortcut: "Cmd Shift [",
        alternate_shortcut: None,
    },
    AgentShortcutDefinition {
        id: "next_thread",
        label: "Next thread",
        default_shortcut: "Cmd Shift ]",
        alternate_shortcut: None,
    },
];

fn command_item(
    id: &str,
    label: &str,
    shortcut: &str,
    alternate_shortcut: Option<String>,
    enabled: bool,
) -> AgentCommandPaletteItem {
    AgentCommandPaletteItem {
        id: id.to_string(),
        label: label.to_string(),
        shortcut: shortcut.to_string(),
        alternate_shortcut,
        enabled,
    }
}

fn shortcut_definition(id: &str) -> Option<AgentShortcutDefinition> {
    AGENT_SHORTCUTS
        .iter()
        .copied()
        .find(|definition| definition.id == id)
}

fn normalize_key_token(token: &str) -> String {
    match token.to_ascii_lowercase().as_str() {
        "cmd" | "command" | "meta" | "super" => "Cmd".to_string(),
        "ctrl" | "control" => "Ctrl".to_string(),
        "alt" | "option" => "Alt".to_string(),
        "shift" => "Shift".to_string(),
        "space" => "Space".to_string(),
        "escape" | "esc" => "Escape".to_string(),
        "enter" | "return" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        "[" | "]" | "\\" | "/" | "." | "," | ";" | "'" | "-" | "=" | "`" => token.to_string(),
        _ if token.len() == 1 => token.to_ascii_uppercase(),
        _ => {
            let mut chars = token.chars();
            match chars.next() {
                Some(first) => {
                    let mut result = first.to_uppercase().collect::<String>();
                    result.push_str(&chars.as_str().to_ascii_lowercase());
                    result
                }
                None => String::new(),
            }
        }
    }
}

pub(crate) fn normalize_shortcut(shortcut: &str) -> Result<String, String> {
    let trimmed = shortcut.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let mut cmd = false;
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key: Option<String> = None;

    for raw_token in trimmed
        .replace('+', " ")
        .split_whitespace()
        .filter(|token| !token.is_empty())
    {
        let token = normalize_key_token(raw_token);
        match token.as_str() {
            "Cmd" => cmd = true,
            "Ctrl" => ctrl = true,
            "Alt" => alt = true,
            "Shift" => shift = true,
            _ => {
                if key.is_some() {
                    return Err(format!("shortcut '{shortcut}' has more than one key"));
                }
                key = Some(token);
            }
        }
    }

    let key = key.ok_or_else(|| format!("shortcut '{shortcut}' is missing a key"))?;
    if !(cmd || ctrl || alt) {
        return Err(format!(
            "shortcut '{shortcut}' must include Cmd, Ctrl, or Alt"
        ));
    }

    let mut parts = Vec::new();
    if cmd {
        parts.push("Cmd".to_string());
    }
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    parts.push(key);
    Ok(parts.join(" "))
}

fn shortcut_for(
    shortcuts: &AgentShortcutSection,
    definition: AgentShortcutDefinition,
) -> Result<String, String> {
    match shortcuts.overrides.get(definition.id) {
        Some(shortcut) => normalize_shortcut(shortcut),
        None => Ok(definition.default_shortcut.to_string()),
    }
}

fn shortcut_settings_inner(
    shortcuts: &AgentShortcutSection,
) -> Result<Vec<AgentShortcutSetting>, String> {
    AGENT_SHORTCUTS
        .iter()
        .copied()
        .map(|definition| {
            let shortcut = shortcut_for(shortcuts, definition)?;
            Ok(AgentShortcutSetting {
                id: definition.id.to_string(),
                label: definition.label.to_string(),
                shortcut,
                alternate_shortcut: definition.alternate_shortcut.map(str::to_string),
                default_shortcut: definition.default_shortcut.to_string(),
            })
        })
        .collect()
}

fn validate_shortcut_settings(settings: &[AgentShortcutSetting]) -> Result<(), String> {
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for setting in settings {
        for shortcut in [
            Some(setting.shortcut.as_str()),
            setting.alternate_shortcut.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|shortcut| !shortcut.is_empty())
        {
            if let Some(existing_id) = seen.insert(shortcut, setting.id.as_str()) {
                return Err(format!(
                    "shortcut '{shortcut}' is used by both '{existing_id}' and '{}'",
                    setting.id
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn present_agent_command_palette_inner(
    request: &AgentCommandPaletteRequest,
    shortcuts: &AgentShortcutSection,
) -> Vec<AgentCommandPaletteItem> {
    let has_multiple_sessions = request.session_count > 1;
    let enabled_by_id = HashMap::from([
        ("new_thread", true),
        ("search_threads", true),
        ("find_in_thread", request.has_active_session),
        ("copy_latest_response", request.has_active_session),
        ("toggle_raw_scrollback", request.has_active_session),
        ("previous_thread", has_multiple_sessions),
        ("next_thread", has_multiple_sessions),
    ]);

    shortcut_settings_inner(shortcuts)
        .unwrap_or_else(|_| shortcut_settings_inner(&AgentShortcutSection::default()).unwrap())
        .into_iter()
        .filter(|setting| setting.id != "command_menu")
        .map(|setting| {
            let enabled = enabled_by_id
                .get(setting.id.as_str())
                .copied()
                .unwrap_or(false);
            command_item(
                &setting.id,
                &setting.label,
                &setting.shortcut,
                setting.alternate_shortcut,
                enabled,
            )
        })
        .collect()
}

pub(crate) fn is_agent_command_enabled_inner(
    request: &AgentCommandPaletteRequest,
    shortcuts: &AgentShortcutSection,
    command_id: &str,
) -> bool {
    if command_id == "command_menu" {
        return true;
    }
    present_agent_command_palette_inner(request, shortcuts)
        .into_iter()
        .find(|item| item.id == command_id)
        .map(|item| item.enabled)
        .unwrap_or(false)
}

fn update_shortcuts_inner(
    current: &AgentShortcutSection,
    updates: Vec<AgentShortcutUpdate>,
) -> Result<AgentShortcutSection, String> {
    let known_ids = AGENT_SHORTCUTS
        .iter()
        .map(|definition| definition.id)
        .collect::<HashSet<_>>();
    let mut overrides = current.overrides.clone();
    for update in updates {
        if !known_ids.contains(update.id.as_str()) {
            return Err(format!("unknown agent shortcut '{}'", update.id));
        }
        let definition = shortcut_definition(&update.id)
            .ok_or_else(|| format!("unknown agent shortcut '{}'", update.id))?;
        let normalized = normalize_shortcut(&update.shortcut)?;
        if normalized == definition.default_shortcut {
            overrides.remove(&update.id);
        } else {
            overrides.insert(update.id, normalized);
        }
    }
    let next = AgentShortcutSection { overrides };
    let settings = shortcut_settings_inner(&next)?;
    validate_shortcut_settings(&settings)?;
    Ok(next)
}

#[tauri::command]
pub fn present_agent_command_palette(
    request: AgentCommandPaletteRequest,
    state: tauri::State<'_, Arc<AppConfig>>,
) -> Vec<AgentCommandPaletteItem> {
    let shortcuts = state
        .get_config()
        .map(|config| config.app.agent_shortcuts)
        .unwrap_or_default();
    present_agent_command_palette_inner(&request, &shortcuts)
}

#[tauri::command]
pub fn is_agent_command_enabled(
    request: AgentCommandEnabledRequest,
    state: tauri::State<'_, Arc<AppConfig>>,
) -> bool {
    let shortcuts = state
        .get_config()
        .map(|config| config.app.agent_shortcuts)
        .unwrap_or_default();
    is_agent_command_enabled_inner(&request.request, &shortcuts, &request.command_id)
}

#[tauri::command]
pub fn get_agent_shortcut_settings(
    state: tauri::State<'_, Arc<AppConfig>>,
) -> Result<Vec<AgentShortcutSetting>, String> {
    let shortcuts = state.get_config()?.app.agent_shortcuts;
    let settings = shortcut_settings_inner(&shortcuts)?;
    validate_shortcut_settings(&settings)?;
    Ok(settings)
}

#[tauri::command]
pub async fn update_agent_shortcut_settings(
    state: tauri::State<'_, Arc<AppConfig>>,
    shortcuts: Vec<AgentShortcutUpdate>,
) -> Result<Vec<AgentShortcutSetting>, String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        app_config.with_config_mut(|config| {
            let next = update_shortcuts_inner(&config.app.agent_shortcuts, shortcuts)?;
            config.app.agent_shortcuts = next;
            shortcut_settings_inner(&config.app.agent_shortcuts).and_then(|settings| {
                validate_shortcut_settings(&settings)?;
                Ok(settings)
            })
        })
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn reset_agent_shortcut_settings(
    state: tauri::State<'_, Arc<AppConfig>>,
) -> Result<Vec<AgentShortcutSetting>, String> {
    let app_config = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        app_config.with_config_mut(|config| {
            config.app.agent_shortcuts = AgentShortcutSection::default();
            shortcut_settings_inner(&config.app.agent_shortcuts)
        })
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presents_thread_commands_with_enabled_state() {
        let result = present_agent_command_palette_inner(
            &AgentCommandPaletteRequest {
                has_active_session: true,
                session_count: 2,
            },
            &AgentShortcutSection::default(),
        );

        assert!(result
            .iter()
            .any(|item| item.id == "new_thread" && item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "find_in_thread" && item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "copy_latest_response" && item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "toggle_raw_scrollback" && item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "previous_thread" && item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "next_thread" && item.enabled));
    }

    #[test]
    fn shortcut_settings_expose_native_command_menu_alternate_shortcut() {
        let settings = shortcut_settings_inner(&AgentShortcutSection::default()).unwrap();
        assert!(settings.iter().any(|item| {
            item.id == "command_menu"
                && item.shortcut == "Cmd K"
                && item.alternate_shortcut.as_deref() == Some("Cmd Shift P")
        }));
    }

    #[test]
    fn disables_session_scoped_commands_without_active_session() {
        let result = present_agent_command_palette_inner(
            &AgentCommandPaletteRequest {
                has_active_session: false,
                session_count: 1,
            },
            &AgentShortcutSection::default(),
        );

        assert!(result
            .iter()
            .any(|item| item.id == "find_in_thread" && !item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "copy_latest_response" && !item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "toggle_raw_scrollback" && !item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "previous_thread" && !item.enabled));
        assert!(result
            .iter()
            .any(|item| item.id == "next_thread" && !item.enabled));
    }

    #[test]
    fn checks_single_command_enabled_state() {
        let shortcuts = AgentShortcutSection::default();
        let request = AgentCommandPaletteRequest {
            has_active_session: true,
            session_count: 1,
        };

        assert!(is_agent_command_enabled_inner(
            &request,
            &shortcuts,
            "toggle_raw_scrollback",
        ));
        assert!(is_agent_command_enabled_inner(
            &request,
            &shortcuts,
            "command_menu",
        ));
        assert!(!is_agent_command_enabled_inner(
            &request,
            &shortcuts,
            "unknown_command",
        ));
    }

    #[test]
    fn normalizes_configured_shortcuts() {
        let next = update_shortcuts_inner(
            &AgentShortcutSection::default(),
            vec![AgentShortcutUpdate {
                id: "new_thread".to_string(),
                shortcut: "ctrl+shift+n".to_string(),
            }],
        )
        .unwrap();
        let settings = shortcut_settings_inner(&next).unwrap();

        assert!(settings
            .iter()
            .any(|item| item.id == "new_thread" && item.shortcut == "Ctrl Shift N"));
    }

    #[test]
    fn rejects_duplicate_shortcuts() {
        let result = update_shortcuts_inner(
            &AgentShortcutSection::default(),
            vec![AgentShortcutUpdate {
                id: "new_thread".to_string(),
                shortcut: "Cmd G".to_string(),
            }],
        );

        assert!(result.is_err());
    }
}
