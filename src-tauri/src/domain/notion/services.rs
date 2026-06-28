#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Spec #1302 keeps Notion branch derivation as a Rust-owned rule without production wiring."
    )
)]
pub(crate) fn notion_branch_name(
    branch_name_property: &str,
    page_id: Option<&str>,
    prefix: Option<&str>,
) -> String {
    let sanitized = sanitize_branch_name_property(branch_name_property);
    if !sanitized.is_empty() {
        return with_prefix(sanitized, prefix);
    }

    if let Some(page_id) = page_id.filter(|value| !value.is_empty()) {
        let short_id: String = page_id.chars().filter(|ch| *ch != '-').take(8).collect();
        return with_prefix(format!("notion/{short_id}"), prefix);
    }

    with_prefix("notion-task".to_string(), prefix)
}

pub(crate) fn notion_task_title_branch_name(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_separator = false;

    for ch in title.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
            previous_was_separator = false;
        } else if !previous_was_separator {
            slug.push('-');
            previous_was_separator = true;
        }
    }

    let slug = slug.trim_matches('-').chars().take(40).collect::<String>();
    format!("feat/{slug}")
}

fn sanitize_branch_name_property(value: &str) -> String {
    let whitespace_collapsed = collapse_whitespace_to_dash(value.trim());
    let allowed_only: String = whitespace_collapsed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-'))
        .collect();
    let dash_collapsed = collapse_repeated_dash(&allowed_only);
    dash_collapsed
        .trim_matches(|ch| matches!(ch, '-' | '/'))
        .to_string()
}

fn collapse_whitespace_to_dash(value: &str) -> String {
    let mut result = String::new();
    let mut previous_was_whitespace = false;
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !previous_was_whitespace {
                result.push('-');
            }
            previous_was_whitespace = true;
        } else {
            result.push(ch);
            previous_was_whitespace = false;
        }
    }
    result
}

fn collapse_repeated_dash(value: &str) -> String {
    let mut result = String::new();
    let mut previous_was_dash = false;
    for ch in value.chars() {
        if ch == '-' {
            if !previous_was_dash {
                result.push(ch);
            }
            previous_was_dash = true;
        } else {
            result.push(ch);
            previous_was_dash = false;
        }
    }
    result
}

fn with_prefix(value: String, prefix: Option<&str>) -> String {
    match prefix.filter(|prefix| !prefix.is_empty()) {
        Some(prefix) if !value.starts_with(prefix) => format!("{prefix}{value}"),
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_valid_branch_names() {
        assert_eq!(
            notion_branch_name("feat/add-login", None, None),
            "feat/add-login"
        );
        assert_eq!(
            notion_branch_name("Fix/Login-Bug", None, None),
            "Fix/Login-Bug"
        );
        assert_eq!(notion_branch_name("PROJ-123", None, None), "PROJ-123");
        assert_eq!(
            notion_branch_name("fix_login_bug", None, None),
            "fix_login_bug"
        );
        assert_eq!(
            notion_branch_name("feat/issues/123", None, None),
            "feat/issues/123"
        );
    }

    #[test]
    fn sanitizes_whitespace_symbols_and_repeated_dashes() {
        assert_eq!(
            notion_branch_name("fix login bug", None, None),
            "fix-login-bug"
        );
        assert_eq!(
            notion_branch_name("feat: add @login!", None, None),
            "feat-add-login"
        );
        assert_eq!(notion_branch_name("fix -- bug", None, None), "fix-bug");
        assert_eq!(notion_branch_name("--fix-bug--", None, None), "fix-bug");
        assert_eq!(
            notion_branch_name("feat/ログイン-fix", None, None),
            "feat/-fix"
        );
    }

    #[test]
    fn falls_back_to_page_id_or_static_name() {
        assert_eq!(
            notion_branch_name("", Some("abcdef12-3456-7890-abcd-ef1234567890"), None),
            "notion/abcdef12"
        );
        assert_eq!(
            notion_branch_name("!@#", Some("ab-cd-ef-12-34-56"), None),
            "notion/abcdef12"
        );
        assert_eq!(notion_branch_name("", None, None), "notion-task");
        assert_eq!(notion_branch_name("!@#$%", None, None), "notion-task");
    }

    #[test]
    fn applies_prefix_when_missing() {
        assert_eq!(
            notion_branch_name("login-bug", None, Some("fix/")),
            "fix/login-bug"
        );
        assert_eq!(
            notion_branch_name("feat/add-login", None, Some("feat/")),
            "feat/add-login"
        );
        assert_eq!(
            notion_branch_name(
                "",
                Some("a1b2c3d4-e5f6-7890-abcd-ef1234567890"),
                Some("feat/")
            ),
            "feat/notion/a1b2c3d4"
        );
        assert_eq!(
            notion_branch_name("", None, Some("fix/")),
            "fix/notion-task"
        );
    }

    #[test]
    fn derives_fallback_branch_from_task_title() {
        assert_eq!(
            notion_task_title_branch_name("Move Notion branch rules"),
            "feat/move-notion-branch-rules"
        );
        assert_eq!(
            notion_task_title_branch_name("BUG: Fix Login!"),
            "feat/bug-fix-login"
        );
        assert_eq!(notion_task_title_branch_name("ログイン"), "feat/");
    }

    #[test]
    fn title_fallback_collapses_separators_and_truncates_slug() {
        assert_eq!(
            notion_task_title_branch_name("fix -- login///bug"),
            "feat/fix-login-bug"
        );
        assert_eq!(
            notion_task_title_branch_name("abcdefghijklmnopqrstuvwxyz1234567890-extra"),
            "feat/abcdefghijklmnopqrstuvwxyz1234567890-ext"
        );
    }
}
