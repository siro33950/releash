pub(crate) mod tests {
    use super::super::*;

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
