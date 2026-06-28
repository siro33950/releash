//! code ユースケースの read model（DTO）。
//!
//! フロントへ返す転送表現を所有する層。Query 経路（QueryService）が読み取り要求ごとに
//! 本 DTO を直接組み立てる。serialize 表現（フィールド名・camelCase・省略）は移行前の
//! 各型と等価に保つ。
//!
//! branch diff のサマリは git2 の diff 結果を denormalize した表示・転送向けモデルで
//! あり domain Entity ではない。Query 経路（[`BranchDiffQuery`](super::code_query_service::BranchDiffQuery)）の
//! gateway 実装がデータソース（git2）から直接組み立てる。

use serde::{Deserialize, Serialize};

use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStatsDto {
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFileDto {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub binary: bool,
    pub stats: DiffStatsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiffSummaryDto {
    pub base_branch: String,
    pub changed_files: Vec<ChangedFileDto>,
    pub stats: DiffStatsDto,
}

// ── hunk / patch / range（Query が domain サービスの算出結果を詰め替えて返す） ──

/// 単一の diff hunk の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HunkDto {
    pub index: u32,
    pub hunk_id: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

/// hunk 内の変更ブロック（Approve 単位）の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeGroupDto {
    pub group_index: u32,
    pub group_id: String,
    pub hunk_index: u32,
    pub new_start: u32,
    pub new_end: u32,
    pub line_offset_start: u32,
    pub line_offset_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_staged: Option<bool>,
}

/// diff hunk 計算結果の転送表現。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunksResultDto {
    pub hunks: Vec<HunkDto>,
    pub change_groups: Vec<ChangeGroupDto>,
}

/// diff-only 表示で折り畳む行範囲の転送表現。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenRangeDto {
    pub start_line: u32,
    pub end_line: u32,
    pub hidden_count: u32,
}

/// Markdown diff-only 表示で可視にする行ブロックの転送表現。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleBlockDto {
    pub start_line: u32,
    pub end_line: u32,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_content: Option<String>,
}

/// Markdown gutter diff range の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffRangeDto {
    pub start_line: u32,
    pub end_line: u32,
    #[serde(rename = "type")]
    pub kind: DiffRangeKindDto,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiffRangeKindDto {
    Added,
    Modified,
    Deleted,
}

/// Markdown split diff row の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SplitRowDto {
    pub left: Option<String>,
    pub right: Option<String>,
    #[serde(rename = "type")]
    pub kind: SplitRowKindDto,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SplitRowKindDto {
    Unchanged,
    Added,
    Removed,
    Modified,
}

/// Markdown inline diff chunk の転送表現。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InlineChunkDto {
    pub content: String,
    #[serde(rename = "type")]
    pub kind: InlineChunkKindDto,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InlineChunkKindDto {
    Unchanged,
    Added,
    Removed,
}

// ── diff_tree（フィールド名は snake_case のまま＝移行前と等価） ──

/// diff ファイルツリーのノードの転送表現。
#[derive(Debug, Clone, Serialize)]
pub struct DiffTreeNodeDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub node_type: String,
    pub status: Option<String>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub children: Vec<DiffTreeNodeDto>,
}

/// ファイルナビゲーション情報の転送表現。
#[derive(Debug, Clone, Serialize)]
pub struct FileNavigationResultDto {
    pub current_index: usize,
    pub total: usize,
    pub prev_file: Option<String>,
    pub next_file: Option<String>,
}

/// review ファイル一覧 read model。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotDto {
    pub version: u64,
    pub stale: bool,
    pub loading: bool,
    pub limited: bool,
    pub base: String,
    pub files: Vec<ReviewFileEntryDto>,
    pub staged_files: Vec<FileStatusDto>,
    pub changed_files: Vec<FileStatusDto>,
    pub diff_stats: Vec<FileDiffStatDto>,
    pub tree: Vec<DiffTreeNodeDto>,
    pub staged_tree: Vec<DiffTreeNodeDto>,
    pub changes_tree: Vec<DiffTreeNodeDto>,
    pub staged_file_count: usize,
    pub changes_file_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileEntryDto {
    pub file_id: String,
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ViewportDto {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ReviewFileViewDto {
    TextDiff(ReviewTextDiffDto),
    Image(ReviewImageDto),
    Binary(ReviewBinaryDto),
    Fallback(ReviewFallbackDto),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewTextDiffDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub original: String,
    pub modified: String,
    pub source: ReviewTextSource,
    pub hunks: Vec<HunkDto>,
    pub change_groups: Vec<ChangeGroupDto>,
    pub limited: bool,
    pub viewport: Option<ViewportDto>,
    pub total_lines: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewTextSource {
    Diff,
    Added,
    Deleted,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewImageDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub original_url: Option<String>,
    pub modified_url: Option<String>,
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBinaryDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub original_url: Option<String>,
    pub modified_url: Option<String>,
    pub original_size: Option<u64>,
    pub modified_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFallbackDto {
    pub version: u64,
    pub stale: bool,
    pub file_id: String,
    pub path: String,
    pub reason: ReviewLimitReasonDto,
    pub total_lines: Option<u32>,
    pub size_bytes: Option<u64>,
    pub hunk_count: Option<u32>,
    pub limited: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReviewLimitReasonDto {
    FileSize,
    LineCount,
    HunkCount,
    Tokenization,
}

#[cfg(test)]
mod code_dto_serialize_tests {
    //! DTO の serialize 表現（フィールド名・camelCase / snake_case・省略）が移行前の各型と
    //! 等価であることを golden で固定する。フロント／リモートが依存する転送契約の回帰防止。
    use super::*;
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
            viewport: Some(ViewportDto {
                start_line: 1,
                end_line: 2,
            }),
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
                "viewport": {"startLine": 1, "endLine": 2},
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
            limited: false,
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

        assert_eq!(v["fileId"], json!(null));
        assert_eq!(v["files"][0]["fileId"], json!("a.rs"));
        assert_eq!(v["stagedFiles"][0]["path"], json!("a.rs"));
        assert_eq!(v["changedFiles"], json!([]));
        assert_eq!(v["stagedTree"], json!([]));
        assert_eq!(v["tree"][0]["node_type"], json!("file"));
    }
}
