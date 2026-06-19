use crate::domain::hooks::value_objects::HooksStatus;

pub fn build_hooks_json(port: u16, token: &str) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\",\"pty_id\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"prompt_submit\" \"$RELEASH_PTY_ID\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "Stop": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\",\"pty_id\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"stop\" \"$RELEASH_PTY_ID\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "Notification": [
                {
                    "matcher": "permission_prompt",
                    "hooks": [{
                        "type": "command",
                        "command": format!(
                            "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\",\"pty_id\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"notification\" \"$RELEASH_PTY_ID\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                        )
                    }]
                },
                {
                    "matcher": "elicitation_dialog",
                    "hooks": [{
                        "type": "command",
                        "command": format!(
                            "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\",\"pty_id\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"notification\" \"$RELEASH_PTY_ID\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                        )
                    }]
                }
            ],
            "PostToolUse": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\",\"pty_id\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"post_tool_use\" \"$RELEASH_PTY_ID\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "PostToolUseFailure": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\",\"pty_id\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"post_tool_use_failure\" \"$RELEASH_PTY_ID\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }],
            "SessionStart": [{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "printf '{{\"worktree_path\":\"%s\",\"event\":\"%s\",\"pty_id\":\"%s\"}}' \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)\" \"session_start\" \"$RELEASH_PTY_ID\" | curl -s -X POST http://localhost:{port}/hooks/agent -H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' -d @- || true"
                    )
                }]
            }]
        }
    })
}

pub fn is_releash_hook_entry(entry: &serde_json::Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|cmd| cmd.contains("/hooks/agent"))
            })
        })
}

pub fn merge_hooks(
    existing: &mut serde_json::Value,
    new_config: &serde_json::Value,
) -> Result<(), String> {
    if let Some(serde_json::Value::Object(new_hooks)) = new_config.get("hooks") {
        let existing_hooks = existing
            .as_object_mut()
            .ok_or("settings.jsonがオブジェクトではありません")?
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}));
        if let serde_json::Value::Object(map) = existing_hooks {
            for (key, new_entries) in new_hooks {
                let existing_entries = map
                    .entry(key.clone())
                    .or_insert_with(|| serde_json::json!([]));

                if let (Some(existing_arr), Some(new_arr)) =
                    (existing_entries.as_array_mut(), new_entries.as_array())
                {
                    for new_entry in new_arr {
                        let new_matcher = new_entry
                            .get("matcher")
                            .and_then(|m| m.as_str())
                            .unwrap_or("");

                        let existing_idx = existing_arr.iter().position(|e| {
                            let matcher_matches =
                                e.get("matcher").and_then(|m| m.as_str()).unwrap_or("")
                                    == new_matcher;
                            matcher_matches && is_releash_hook_entry(e)
                        });

                        match existing_idx {
                            Some(idx) => existing_arr[idx] = new_entry.clone(),
                            None => existing_arr.push(new_entry.clone()),
                        }
                    }
                }
            }
        } else {
            *existing_hooks = serde_json::Value::Object(new_hooks.clone());
        }
    }
    Ok(())
}

pub fn detect_hooks_status(
    parsed_settings: &serde_json::Value,
    hook_port: u16,
    token: &str,
) -> HooksStatus {
    let port_str = format!("localhost:{hook_port}");
    let hooks_str = parsed_settings
        .get("hooks")
        .map(|h| h.to_string())
        .unwrap_or_default();

    if !hooks_str.contains(&port_str) {
        return HooksStatus::NotConfigured;
    }

    if !hooks_str.contains(token) {
        return HooksStatus::TokenMismatch;
    }

    HooksStatus::Active
}

#[cfg(test)]
mod tests {
    use super::*;

    fn releash_hook_entry(matcher: &str, port: u16) -> serde_json::Value {
        serde_json::json!({
            "matcher": matcher,
            "hooks": [{
                "type": "command",
                "command": format!(
                    "curl -s -X POST http://localhost:{port}/hooks/agent -H 'Content-Type: application/json' -d '{{}}' || true"
                )
            }]
        })
    }

    fn user_hook_entry(matcher: &str, command: &str) -> serde_json::Value {
        serde_json::json!({
            "matcher": matcher,
            "hooks": [{
                "type": "command",
                "command": command
            }]
        })
    }

    #[test]
    fn is_releash_hook_entry_identifies_releash_hooks() {
        assert!(is_releash_hook_entry(&releash_hook_entry("", 19700)));
        assert!(!is_releash_hook_entry(&user_hook_entry("", "echo hello")));
    }

    #[test]
    fn merge_hooks_preserves_user_hooks() {
        let user_entry =
            user_hook_entry("permission_prompt", "notify-send 'Claude needs permission'");
        let mut existing = serde_json::json!({
            "hooks": {
                "Notification": [user_entry.clone()]
            }
        });

        let new_config = serde_json::json!({
            "hooks": {
                "Notification": [
                    releash_hook_entry("permission_prompt", 19700),
                    releash_hook_entry("elicitation_dialog", 19700),
                ]
            }
        });

        merge_hooks(&mut existing, &new_config).unwrap();

        let entries = existing["hooks"]["Notification"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], user_entry);
    }

    #[test]
    fn detects_token_mismatch() {
        let settings = build_hooks_json(19700, "old");
        assert_eq!(
            detect_hooks_status(&settings, 19700, "new"),
            HooksStatus::TokenMismatch
        );
    }
}
