use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffFileEntry {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffTreeNode {
    pub name: String,
    pub path: String,
    pub node_type: String,
    pub status: Option<String>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub children: Vec<DiffTreeNode>,
}

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
mod tests {
    use super::*;

    fn entry(path: &str, status: &str) -> DiffFileEntry {
        DiffFileEntry {
            path: path.to_string(),
            status: status.to_string(),
            additions: 0,
            deletions: 0,
        }
    }

    fn entry_with_stats(path: &str, status: &str, additions: u32, deletions: u32) -> DiffFileEntry {
        DiffFileEntry {
            path: path.to_string(),
            status: status.to_string(),
            additions,
            deletions,
        }
    }

    #[test]
    fn empty_input_returns_empty_tree() {
        let result = build_tree(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn single_file_at_root() {
        let entries = vec![entry("README.md", "modified")];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "README.md");
        assert_eq!(tree[0].node_type, "file");
        assert_eq!(tree[0].status.as_deref(), Some("modified"));
        assert_eq!(tree[0].path, "README.md");
    }

    #[test]
    fn single_child_directory_collapsing() {
        let entries = vec![entry("src/components/panels/Review.tsx", "new")];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        // All single-child folders should be collapsed
        assert_eq!(tree[0].name, "src/components/panels");
        assert_eq!(tree[0].node_type, "folder");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "Review.tsx");
        assert_eq!(tree[0].children[0].node_type, "file");
    }

    #[test]
    fn multiple_files_in_same_directory() {
        let entries = vec![
            entry("src/hooks/useA.ts", "modified"),
            entry("src/hooks/useB.ts", "new"),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "src/hooks");
        assert_eq!(tree[0].node_type, "folder");
        assert_eq!(tree[0].children.len(), 2);
    }

    #[test]
    fn no_collapsing_when_multiple_children() {
        let entries = vec![
            entry("src/a.ts", "modified"),
            entry("src/b.ts", "deleted"),
            entry("lib/c.ts", "new"),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 2); // lib, src
                                   // Both should be folders with file children
        for node in &tree {
            assert_eq!(node.node_type, "folder");
        }
    }

    #[test]
    fn deep_nesting() {
        let entries = vec![
            entry("a/b/c/d/e.txt", "modified"),
            entry("a/b/c/d/f.txt", "new"),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        // a/b/c/d should be collapsed into single folder
        assert_eq!(tree[0].name, "a/b/c/d");
        assert_eq!(tree[0].node_type, "folder");
        assert_eq!(tree[0].children.len(), 2);
    }

    #[test]
    fn mixed_depths() {
        let entries = vec![
            entry("Cargo.toml", "modified"),
            entry("src/main.rs", "modified"),
            entry("src/git/diff.rs", "new"),
        ];
        let tree = build_tree(entries);
        // Root level: Cargo.toml (file) + src (folder)
        assert_eq!(tree.len(), 2);

        let cargo = tree.iter().find(|n| n.name == "Cargo.toml").unwrap();
        assert_eq!(cargo.node_type, "file");

        let src = tree.iter().find(|n| n.name == "src").unwrap();
        assert_eq!(src.node_type, "folder");
        // src has two children: main.rs and git (folder)
        assert_eq!(src.children.len(), 2);
    }

    #[test]
    fn stats_propagated_to_tree_node() {
        let entries = vec![entry_with_stats("file.rs", "modified", 10, 3)];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].additions, Some(10));
        assert_eq!(tree[0].deletions, Some(3));
    }

    #[test]
    fn folder_has_no_stats() {
        let entries = vec![
            entry_with_stats("src/a.ts", "modified", 5, 2),
            entry_with_stats("src/b.ts", "new", 20, 0),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree[0].node_type, "folder");
        assert_eq!(tree[0].additions, None);
        assert_eq!(tree[0].deletions, None);
        // Children have stats
        assert_eq!(tree[0].children[0].additions, Some(5));
        assert_eq!(tree[0].children[1].additions, Some(20));
    }

    #[test]
    fn file_directory_replacement_preserves_both() {
        // Scenario: "foo" is deleted (file) and "foo/bar.rs" is added (nested in dir)
        let entries = vec![entry("foo", "deleted"), entry("foo/bar.rs", "new")];
        let tree = build_tree(entries);

        let has_deleted_file = tree
            .iter()
            .any(|n| n.path == "foo" && n.node_type == "file");
        assert!(
            has_deleted_file,
            "deleted file entry 'foo' should be present"
        );

        let has_nested = tree
            .iter()
            .any(|n| n.node_type == "folder" && n.children.iter().any(|c| c.path == "foo/bar.rs"));
        assert!(
            has_nested,
            "nested added entry 'foo/bar.rs' should be present"
        );
    }
}
