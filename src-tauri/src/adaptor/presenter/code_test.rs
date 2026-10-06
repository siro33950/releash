pub(crate) mod protocol_code_tests {
    //! フロント境界（Tauri コマンド引数）の deserialize 表現と `into_domain()` 変換を固定する。
    //! 型ごとの camelCase / snake_case の別、`is_staged` の省略時 default、再帰変換が移行前と
    //! 等価にマッピングされることを担保する。
    use super::super::*;

    #[test]
    fn test_hunk_inputはcamelcaseを受理しdomainへ変換する() {
        let json = r#"{"index":0,"oldStart":1,"oldLines":2,"newStart":3,"newLines":4,"lines":["-a","+b"]}"#;
        let input: HunkInput = serde_json::from_str(json).unwrap();
        let h = input.into_domain();
        assert_eq!(h.index, 0);
        assert_eq!(h.hunk_id, "");
        assert_eq!(h.old_start, 1);
        assert_eq!(h.old_lines, 2);
        assert_eq!(h.new_start, 3);
        assert_eq!(h.new_lines, 4);
        assert_eq!(h.lines, vec!["-a".to_string(), "+b".to_string()]);
    }

    #[test]
    fn test_markdown_diff_side_inputはlowercaseを受理する() {
        let modified: MarkdownDiffSideInput = serde_json::from_str(r#""modified""#).unwrap();
        let original: MarkdownDiffSideInput = serde_json::from_str(r#""original""#).unwrap();

        assert!(matches!(modified.into_usecase(), DiffSide::Modified));
        assert!(matches!(original.into_usecase(), DiffSide::Original));
    }

    #[test]
    fn test_diff_file_entry_inputはsnake_caseを受理する() {
        let json = r#"{"path":"f.rs","status":"modified","additions":3,"deletions":1}"#;
        let input: DiffFileEntryInput = serde_json::from_str(json).unwrap();
        let d = input.into_domain();
        assert_eq!(d.path, "f.rs");
        assert_eq!(d.status, "modified");
        assert_eq!(d.additions, 3);
        assert_eq!(d.deletions, 1);
    }

    #[test]
    fn test_diff_tree_node_inputはsnake_caseを子ノード再帰で変換する() {
        let json = r#"{
            "id":"p","name":"a","path":"a","node_type":"directory",
            "status":null,"additions":null,"deletions":null,
            "children":[
                {"id":"c","name":"child","path":"a/child","node_type":"file",
                 "status":"modified","additions":1,"deletions":2,"children":[]}
            ]
        }"#;
        let input: DiffTreeNodeInput = serde_json::from_str(json).unwrap();
        let d = input.into_domain();
        assert_eq!(d.node_type, "directory");
        assert_eq!(d.status, None);
        assert_eq!(d.children.len(), 1);
        assert_eq!(d.children[0].node_type, "file");
        assert_eq!(d.children[0].status, Some("modified".to_string()));
        assert_eq!(d.children[0].additions, Some(1));
        assert_eq!(d.children[0].deletions, Some(2));
    }

    #[test]
    fn test_review_group_action_inputはcamelcaseを受理する() {
        let json = r#"{
            "worktreePath": "/repo",
            "path": "src/main.rs",
            "section": "changes",
            "base": "head",
            "groupId": "g:abc:0"
        }"#;

        let input: ReviewGroupActionInput = serde_json::from_str(json).unwrap();

        assert_eq!(input.worktree_path, "/repo");
        assert_eq!(input.path, "src/main.rs");
        assert_eq!(input.section, "changes");
        assert_eq!(input.base, "head");
        assert_eq!(input.group_id, "g:abc:0");
    }
}
