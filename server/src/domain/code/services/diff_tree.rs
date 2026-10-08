//! diff ファイルツリーの構築とナビゲーション算出（純粋ロジック）。

use std::collections::{BTreeMap, HashSet};

use crate::domain::code::value_objects::{DiffFileEntry, DiffTreeNode, FileNavigationResult};

/// Intermediate tree used during construction.
struct TreeBuilder {
    children: BTreeMap<String, TreeBuilder>,
    /// Present only for leaf (file) nodes.
    file: Option<(String, String, u32, u32)>, // (full_path, status, additions, deletions)
}

impl TreeBuilder {
    fn new() -> Self {
        Self {
            children: BTreeMap::new(),
            file: None,
        }
    }

    fn insert(
        &mut self,
        segments: &[&str],
        full_path: &str,
        status: &str,
        additions: u32,
        deletions: u32,
    ) {
        if segments.is_empty() {
            return;
        }
        if segments.len() == 1 {
            let child = self
                .children
                .entry(segments[0].to_string())
                .or_insert_with(TreeBuilder::new);
            child.file = Some((
                full_path.to_string(),
                status.to_string(),
                additions,
                deletions,
            ));
            return;
        }
        let child = self
            .children
            .entry(segments[0].to_string())
            .or_insert_with(TreeBuilder::new);
        child.insert(&segments[1..], full_path, status, additions, deletions);
    }

    fn into_nodes(self, parent_path: &str) -> Vec<DiffTreeNode> {
        let mut nodes = Vec::new();
        for (name, mut builder) in self.children {
            let path = if parent_path.is_empty() {
                name.clone()
            } else {
                format!("{parent_path}/{name}")
            };

            let has_children = !builder.children.is_empty();
            // Take file info before consuming builder via into_nodes
            let file_info = builder.file.take();

            if has_children {
                // Folder node (or file↔directory replacement) — recurse then collapse
                let children = builder.into_nodes(&path);
                let node = collapse_single_child_folder(name.clone(), path.clone(), children);
                nodes.push(node);
            }

            if let Some((full_path, status, additions, deletions)) = file_info {
                // Leaf: file node (may coexist with folder when file↔directory replacement)
                nodes.push(DiffTreeNode {
                    id: format!("file:{full_path}"),
                    name,
                    path: full_path,
                    node_type: "file".to_string(),
                    status: Some(status),
                    additions: Some(additions),
                    deletions: Some(deletions),
                    children: vec![],
                });
            }
        }
        nodes
    }
}

/// If a folder has exactly one child and that child is also a folder,
/// merge them into a single node with a combined name (e.g. "src/components").
fn collapse_single_child_folder(
    name: String,
    path: String,
    children: Vec<DiffTreeNode>,
) -> DiffTreeNode {
    if children.len() == 1 && children[0].node_type == "folder" {
        let child = children.into_iter().next().unwrap();
        let merged_name = format!("{name}/{}", child.name);
        // Recursively collapse in case of deeper single-child chains
        collapse_single_child_folder(merged_name, child.path, child.children)
    } else {
        DiffTreeNode {
            id: format!("folder:{path}"),
            name,
            path,
            node_type: "folder".to_string(),
            status: None,
            additions: None,
            deletions: None,
            children,
        }
    }
}

/// Flatten tree nodes into an ordered list of unique file paths (depth-first).
/// Duplicates are skipped (keeps first occurrence), which is important when
/// the input contains multiple trees (e.g. staged + unstaged combined).
fn flatten_file_paths(nodes: &[DiffTreeNode]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    collect_file_paths(nodes, &mut paths, &mut seen);
    paths
}

fn collect_file_paths(nodes: &[DiffTreeNode], paths: &mut Vec<String>, seen: &mut HashSet<String>) {
    for node in nodes {
        if node.node_type == "file" && seen.insert(node.path.clone()) {
            paths.push(node.path.clone());
        }
        if !node.children.is_empty() {
            collect_file_paths(&node.children, paths, seen);
        }
    }
}

/// Compute file navigation info (current index, total, prev/next file)
/// from a hierarchical tree and the currently selected file path.
pub fn get_file_navigation(tree: &[DiffTreeNode], current_file: &str) -> FileNavigationResult {
    let files = flatten_file_paths(tree);
    let total = files.len();

    let current_pos = files.iter().position(|p| p == current_file);

    match current_pos {
        Some(idx) => FileNavigationResult {
            current_index: idx,
            total,
            prev_file: if idx > 0 {
                Some(files[idx - 1].clone())
            } else {
                None
            },
            next_file: if idx + 1 < total {
                Some(files[idx + 1].clone())
            } else {
                None
            },
        },
        None => FileNavigationResult {
            current_index: 0,
            total,
            prev_file: None,
            next_file: None,
        },
    }
}

/// Build a directory tree from a flat list of file entries.
///
/// Single-child directories are automatically collapsed
/// (e.g. `src` → `components` → `panels` becomes `src/components/panels`).
pub fn build_tree(entries: Vec<DiffFileEntry>) -> Vec<DiffTreeNode> {
    let mut root = TreeBuilder::new();
    for entry in &entries {
        let segments: Vec<&str> = entry.path.split('/').collect();
        root.insert(
            &segments,
            &entry.path,
            &entry.status,
            entry.additions,
            entry.deletions,
        );
    }
    root.into_nodes("")
}

#[cfg(test)]
#[path = "diff_tree_test.rs"]
mod diff_tree_tests;
