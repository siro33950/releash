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
    fn test_taskのbranch名_未指定ならtask_idを使う() {
        // Given
        let first = "abcdef12-3456-7890-abcd-ef1234567890";
        let second = "abcdef12-3456-7890-abcd-ef1234567891";

        // When / Then
        assert_eq!(notion_task_branch_name("", first), format!("feat/{first}"));
        assert_ne!(
            notion_task_branch_name("", first),
            notion_task_branch_name("", second)
        );
        assert_eq!(
            notion_task_branch_name("fix login bug", first),
            "fix login bug"
        );
    }
}
