//! code 責務の Tauri コマンド引数として受理する入力メッセージ型。
//!
//! domain の値オブジェクトは serde 非依存のため、フロントから受け取る転送表現を本型で
//! 受理し、`into_domain()` で対応するドメイン値オブジェクトへ変換する。フィールド名・
//! camelCase は移行前と等価に保つ（フロントは変更しない）。

use serde::Deserialize;

use crate::domain::code::{ChangeGroup, DiffFileEntry, DiffTreeNode, Hunk};

/// `generate_group_patch` / `compute_hidden_ranges` のコマンド引数として受け取る hunk。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkInput {
    pub index: u32,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

impl HunkInput {
    pub fn into_domain(self) -> Hunk {
        Hunk {
            index: self.index,
            old_start: self.old_start,
            old_lines: self.old_lines,
            new_start: self.new_start,
            new_lines: self.new_lines,
            lines: self.lines,
        }
    }
}

/// `generate_group_patch` のコマンド引数として受け取る change group。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeGroupInput {
    pub group_index: u32,
    pub hunk_index: u32,
    pub new_start: u32,
    pub new_end: u32,
    pub line_offset_start: u32,
    pub line_offset_end: u32,
    #[serde(default)]
    pub is_staged: Option<bool>,
}

impl ChangeGroupInput {
    pub fn into_domain(self) -> ChangeGroup {
        ChangeGroup {
            group_index: self.group_index,
            hunk_index: self.hunk_index,
            new_start: self.new_start,
            new_end: self.new_end,
            line_offset_start: self.line_offset_start,
            line_offset_end: self.line_offset_end,
            is_staged: self.is_staged,
        }
    }
}

/// `build_diff_file_tree` のコマンド引数として受け取るフラットなファイルエントリ。
/// フィールド名は snake_case（移行前と等価）。
#[derive(Debug, Clone, Deserialize)]
pub struct DiffFileEntryInput {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

impl DiffFileEntryInput {
    pub fn into_domain(self) -> DiffFileEntry {
        DiffFileEntry {
            path: self.path,
            status: self.status,
            additions: self.additions,
            deletions: self.deletions,
        }
    }
}

/// `get_file_navigation` のコマンド引数として受け取る diff ツリーノード（再帰）。
/// フィールド名は snake_case（移行前と等価）。
#[derive(Debug, Clone, Deserialize)]
pub struct DiffTreeNodeInput {
    pub id: String,
    pub name: String,
    pub path: String,
    pub node_type: String,
    pub status: Option<String>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub children: Vec<DiffTreeNodeInput>,
}

impl DiffTreeNodeInput {
    pub fn into_domain(self) -> DiffTreeNode {
        DiffTreeNode {
            id: self.id,
            name: self.name,
            path: self.path,
            node_type: self.node_type,
            status: self.status,
            additions: self.additions,
            deletions: self.deletions,
            children: self.children.into_iter().map(Self::into_domain).collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSnapshotInput {
    pub worktree_path: String,
    pub base: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileViewInput {
    pub worktree_path: String,
    pub target: ReviewTargetInput,
    pub section: String,
    pub base: String,
    #[serde(default)]
    pub snapshot_version: Option<u64>,
    pub viewport: Option<ViewportInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "by", content = "value")]
pub enum ReviewTargetInput {
    FileId(String),
    Path(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportInput {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewGroupActionInput {
    pub worktree_path: String,
    pub path: String,
    pub section: String,
    pub base: String,
    pub group_index: u32,
    pub snapshot_version: u64,
}

#[cfg(test)]
mod protocol_code_tests {
    //! フロント境界（Tauri コマンド引数）の deserialize 表現と `into_domain()` 変換を固定する。
    //! 型ごとの camelCase / snake_case の別、`is_staged` の省略時 default、再帰変換が移行前と
    //! 等価にマッピングされることを担保する。
    use super::*;

    #[test]
    fn test_hunk_inputはcamelcaseを受理しdomainへ変換する() {
        let json = r#"{"index":0,"oldStart":1,"oldLines":2,"newStart":3,"newLines":4,"lines":["-a","+b"]}"#;
        let input: HunkInput = serde_json::from_str(json).unwrap();
        let h = input.into_domain();
        assert_eq!(h.index, 0);
        assert_eq!(h.old_start, 1);
        assert_eq!(h.old_lines, 2);
        assert_eq!(h.new_start, 3);
        assert_eq!(h.new_lines, 4);
        assert_eq!(h.lines, vec!["-a".to_string(), "+b".to_string()]);
    }

    #[test]
    fn test_change_group_input_is_stagedの省略_null_true_false() {
        let base = r#""groupIndex":0,"hunkIndex":1,"newStart":2,"newEnd":3,"lineOffsetStart":4,"lineOffsetEnd":5"#;

        // 省略時は #[serde(default)] により None。
        let omitted = format!("{{{base}}}");
        let i: ChangeGroupInput = serde_json::from_str(&omitted).unwrap();
        assert_eq!(i.into_domain().is_staged, None);

        // 明示 null も None。
        let null = format!("{{{base},\"isStaged\":null}}");
        let i: ChangeGroupInput = serde_json::from_str(&null).unwrap();
        assert_eq!(i.into_domain().is_staged, None);

        // true / false はそのまま。
        let t = format!("{{{base},\"isStaged\":true}}");
        let i: ChangeGroupInput = serde_json::from_str(&t).unwrap();
        let d = i.into_domain();
        assert_eq!(d.is_staged, Some(true));
        assert_eq!(d.group_index, 0);
        assert_eq!(d.line_offset_end, 5);

        let f = format!("{{{base},\"isStaged\":false}}");
        let i: ChangeGroupInput = serde_json::from_str(&f).unwrap();
        assert_eq!(i.into_domain().is_staged, Some(false));
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
    fn test_review_file_view_inputはtarget_file_idとviewportを受理する() {
        let json = r#"{
            "worktreePath": "/repo",
            "target": {"by": "fileId", "value": "src/main.rs"},
            "section": "changes",
            "base": "head",
            "snapshotVersion": 12,
            "viewport": {"startLine": 3, "endLine": 8}
        }"#;

        let input: ReviewFileViewInput = serde_json::from_str(json).unwrap();

        assert_eq!(input.worktree_path, "/repo");
        match input.target {
            ReviewTargetInput::FileId(value) => assert_eq!(value, "src/main.rs"),
            ReviewTargetInput::Path(_) => panic!("expected fileId target"),
        }
        assert_eq!(input.snapshot_version, Some(12));
        assert_eq!(input.viewport.unwrap().start_line, 3);
    }

    #[test]
    fn test_review_group_action_inputはcamelcaseを受理する() {
        let json = r#"{
            "worktreePath": "/repo",
            "path": "src/main.rs",
            "section": "changes",
            "base": "head",
            "groupIndex": 2,
            "snapshotVersion": 12
        }"#;

        let input: ReviewGroupActionInput = serde_json::from_str(json).unwrap();

        assert_eq!(input.worktree_path, "/repo");
        assert_eq!(input.path, "src/main.rs");
        assert_eq!(input.section, "changes");
        assert_eq!(input.base, "head");
        assert_eq!(input.group_index, 2);
        assert_eq!(input.snapshot_version, 12);
    }

    #[test]
    fn test_review_snapshot_inputはcamelcaseを受理する() {
        let input: ReviewSnapshotInput =
            serde_json::from_str(r#"{"worktreePath":"/repo","base":"branch-base"}"#).unwrap();

        assert_eq!(input.worktree_path, "/repo");
        assert_eq!(input.base, "branch-base");
    }
}
