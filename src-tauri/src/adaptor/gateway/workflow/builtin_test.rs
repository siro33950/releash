pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::{NodeKind, SessionPermission};

    #[test]
    fn pr_review_import_requires_all_graphql_pages() {
        let instruction =
            get_builtin_facet(FacetKind::Instruction, "import_pr_review_comments").unwrap();

        assert!(instruction.contains("reviewThreads(first:, after: endCursor)"));
        assert!(instruction.contains("comments(first:, after: endCursor)"));
        assert!(instruction.contains("pageInfo.hasNextPage"));
        assert!(instruction.contains("pageInfo.endCursor"));
        assert!(
            instruction.contains("全件取得できない場合は、取得できない状況と原因を具体的に提示し")
        );
        assert!(instruction.contains("解決するまで完了を提出しない"));
    }

    #[test]
    fn test_builtin_workflow_sessionは明示したclaude_providerを使う() {
        let source = builtin_workflow_source("03_full-review").unwrap();

        assert!(source.contains("provider: claude"));
    }

    #[test]
    fn test_builtin_workflow_8本58sessionのpermissionは全件autoである() {
        let mut session_count = 0;

        for entry in BUILTINS {
            let name = entry.filename.strip_suffix(".yml").unwrap();
            let workflow = load_builtin_workflow_resolved(name).unwrap().unwrap();
            for node in workflow.nodes {
                let NodeKind::Session(session) = node.kind else {
                    continue;
                };
                session_count += 1;
                assert_eq!(
                    session.permission,
                    Some(SessionPermission::Auto),
                    "{}:{}",
                    entry.filename,
                    node.name
                );
            }
        }

        assert_eq!(session_count, 58);
    }
}
