pub(crate) mod code_dto_serialize_tests {
    //! DTO の serialize 表現（フィールド名・camelCase / snake_case・省略）が移行前の各型と
    //! 等価であることを golden で固定する。フロント／リモートが依存する転送契約の回帰防止。
    use super::super::*;
    use serde_json::json;

    #[test]
    fn test_hunk_dtoはcamelcaseで出力する() {
        let dto = HunkDto {
            index: 0,
            hunk_id: "h:abc:0".to_string(),
            old_start: 1,
            old_lines: 2,
            new_start: 3,
            new_lines: 4,
            lines: vec!["-a".to_string(), "+b".to_string()],
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({
                "index": 0,
                "hunkId": "h:abc:0",
                "oldStart": 1,
                "oldLines": 2,
                "newStart": 3,
                "newLines": 4,
                "lines": ["-a", "+b"]
            })
        );
    }

    #[test]
    fn test_change_group_dto_is_staged_noneは省略する() {
        let dto = ChangeGroupDto {
            group_index: 0,
            group_id: "g:abc:0".to_string(),
            hunk_index: 1,
            new_start: 2,
            new_end: 3,
            line_offset_start: 4,
            line_offset_end: 5,
            is_staged: None,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            v,
            json!({
                "groupIndex": 0,
                "groupId": "g:abc:0",
                "hunkIndex": 1,
                "newStart": 2,
                "newEnd": 3,
                "lineOffsetStart": 4,
                "lineOffsetEnd": 5
            })
        );
        assert!(v.get("isStaged").is_none());
    }

    #[test]
    fn test_change_group_dto_is_staged_someは出力する() {
        let dto = ChangeGroupDto {
            group_index: 0,
            group_id: "g:abc:0".to_string(),
            hunk_index: 1,
            new_start: 2,
            new_end: 3,
            line_offset_start: 4,
            line_offset_end: 5,
            is_staged: Some(true),
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v.get("isStaged"), Some(&json!(true)));
    }

    #[test]
    fn test_hidden_range_dtoはcamelcaseで出力する() {
        let dto = HiddenRangeDto {
            start_line: 1,
            end_line: 2,
            hidden_count: 3,
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({"startLine": 1, "endLine": 2, "hiddenCount": 3})
        );
    }

    #[test]
    fn test_visible_block_dto_deleted_contentの省略と出力() {
        let none = VisibleBlockDto {
            start_line: 1,
            end_line: 2,
            content: "x".to_string(),
            deleted_content: None,
        };
        let v = serde_json::to_value(&none).unwrap();
        assert_eq!(v, json!({"startLine": 1, "endLine": 2, "content": "x"}));
        assert!(v.get("deletedContent").is_none());

        let some = VisibleBlockDto {
            start_line: 1,
            end_line: 2,
            content: "x".to_string(),
            deleted_content: Some("d".to_string()),
        };
        assert_eq!(
            serde_json::to_value(&some).unwrap().get("deletedContent"),
            Some(&json!("d"))
        );
    }

    #[test]
    fn test_markdown_diff_range_dtoは既存frontend形で出力する() {
        let dto = DiffRangeDto {
            start_line: 2,
            end_line: 4,
            kind: DiffRangeKindDto::Modified,
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({"startLine": 2, "endLine": 4, "type": "modified"})
        );

        let deleted = DiffRangeDto {
            start_line: 5,
            end_line: 6,
            kind: DiffRangeKindDto::Deleted,
        };
        assert_eq!(
            serde_json::to_value(&deleted).unwrap(),
            json!({"startLine": 5, "endLine": 6, "type": "deleted"})
        );
    }

    #[test]
    fn test_markdown_split_row_dtoはtypeとnullable_sideを出力する() {
        let dto = SplitRowDto {
            left: None,
            right: Some("new\n".to_string()),
            kind: SplitRowKindDto::Added,
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({"left": null, "right": "new\n", "type": "added"})
        );
    }

    #[test]
    fn test_markdown_inline_chunk_dtoはtypeを出力する() {
        let dto = InlineChunkDto {
            content: "old\n".to_string(),
            kind: InlineChunkKindDto::Removed,
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({"content": "old\n", "type": "removed"})
        );
    }

    #[test]
    fn test_diff_hunks_result_dtoはcamelcaseで出力する() {
        let dto = DiffHunksResultDto {
            hunks: vec![],
            change_groups: vec![],
        };
        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({"hunks": [], "changeGroups": []})
        );
    }

    #[test]
    fn test_diff_tree_node_dtoはsnake_caseで再帰出力する() {
        let child = DiffTreeNodeDto {
            id: "c".to_string(),
            name: "child".to_string(),
            path: "a/child".to_string(),
            node_type: "file".to_string(),
            status: Some("modified".to_string()),
            additions: Some(1),
            deletions: Some(2),
            children: vec![],
        };
        let parent = DiffTreeNodeDto {
            id: "p".to_string(),
            name: "a".to_string(),
            path: "a".to_string(),
            node_type: "directory".to_string(),
            status: None,
            additions: None,
            deletions: None,
            children: vec![child],
        };
        let v = serde_json::to_value(&parent).unwrap();
        // フィールド名は snake_case（移行前と等価）、children は再帰。
        assert_eq!(v["node_type"], json!("directory"));
        assert_eq!(v["status"], json!(null));
        assert_eq!(v["children"][0]["node_type"], json!("file"));
        assert_eq!(v["children"][0]["status"], json!("modified"));
        assert_eq!(v["children"][0]["additions"], json!(1));
    }

    #[test]
    fn test_branch_diff_summary_dtoはsnake_caseで出力する() {
        let dto = BranchDiffSummaryDto {
            base_branch: "main".to_string(),
            changed_files: vec![ChangedFileDto {
                path: "f.rs".to_string(),
                old_path: Some("g.rs".to_string()),
                status: "renamed".to_string(),
                binary: false,
                stats: DiffStatsDto {
                    additions: 1,
                    deletions: 2,
                },
            }],
            stats: DiffStatsDto {
                additions: 1,
                deletions: 2,
            },
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["base_branch"], json!("main"));
        assert_eq!(v["changed_files"][0]["old_path"], json!("g.rs"));
        assert_eq!(v["changed_files"][0]["stats"]["additions"], json!(1));
        assert_eq!(v["changed_files"][0]["binary"], json!(false));
    }

    #[test]
    fn test_review_file_view_dtoはkindタグでcamelcase出力する() {
        let dto = ReviewFileViewDto::TextDiff(ReviewTextDiffDto {
            version: 4,
            stale: false,
            file_id: "src/main.rs".to_string(),
            path: "src/main.rs".to_string(),
            original: "old".to_string(),
            modified: "new".to_string(),
            source: ReviewTextSource::Diff,
            hunks: vec![HunkDto {
                index: 0,
                hunk_id: "h:old-new:0".to_string(),
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: vec!["-old".to_string(), "+new".to_string()],
            }],
            change_groups: vec![ChangeGroupDto {
                group_index: 0,
                group_id: "g:old-new:0".to_string(),
                hunk_index: 0,
                new_start: 1,
                new_end: 1,
                line_offset_start: 0,
                line_offset_end: 1,
                is_staged: None,
            }],
            limited: false,
            total_lines: 2,
        });

        assert_eq!(
            serde_json::to_value(&dto).unwrap(),
            json!({
                "kind": "textDiff",
                "version": 4,
                "stale": false,
                "fileId": "src/main.rs",
                "path": "src/main.rs",
                "original": "old",
                "modified": "new",
                "source": "diff",
                "hunks": [{
                    "index": 0,
                    "hunkId": "h:old-new:0",
                    "oldStart": 1,
                    "oldLines": 1,
                    "newStart": 1,
                    "newLines": 1,
                    "lines": ["-old", "+new"]
                }],
                "changeGroups": [{
                    "groupIndex": 0,
                    "groupId": "g:old-new:0",
                    "hunkIndex": 0,
                    "newStart": 1,
                    "newEnd": 1,
                    "lineOffsetStart": 0,
                    "lineOffsetEnd": 1
                }],
                "limited": false,
                "totalLines": 2
            })
        );
    }

    #[test]
    fn test_review_snapshot_dtoはcamelcaseと既存tree_snakecaseを混在保持する() {
        let dto = ReviewSnapshotDto {
            version: 4,
            stale: false,
            loading: false,
            base: "head".to_string(),
            files: vec![ReviewFileEntryDto {
                file_id: "a.rs".to_string(),
                path: "a.rs".to_string(),
                index_status: "modified".to_string(),
                worktree_status: "none".to_string(),
                additions: 1,
                deletions: 2,
            }],
            staged_files: vec![FileStatusDto {
                path: "a.rs".to_string(),
                index_status: "modified".to_string(),
                worktree_status: "none".to_string(),
            }],
            changed_files: Vec::new(),
            diff_stats: Vec::new(),
            tree: vec![DiffTreeNodeDto {
                id: "a.rs".to_string(),
                name: "a.rs".to_string(),
                path: "a.rs".to_string(),
                node_type: "file".to_string(),
                status: Some("modified".to_string()),
                additions: Some(1),
                deletions: Some(2),
                children: Vec::new(),
            }],
            staged_tree: Vec::new(),
            changes_tree: Vec::new(),
            staged_file_count: 1,
            changes_file_count: 0,
        };
        let v = serde_json::to_value(&dto).unwrap();

        assert!(v.get("limited").is_none());
        assert_eq!(v["fileId"], json!(null));
        assert_eq!(v["files"][0]["fileId"], json!("a.rs"));
        assert_eq!(v["stagedFiles"][0]["path"], json!("a.rs"));
        assert_eq!(v["changedFiles"], json!([]));
        assert_eq!(v["stagedTree"], json!([]));
        assert_eq!(v["tree"][0]["node_type"], json!("file"));
    }
}
