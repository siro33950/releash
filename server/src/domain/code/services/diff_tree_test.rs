pub(crate) mod diff_tree_service_tests {
    use super::super::*;

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
    fn test_ツリー構築_空入力は空ツリー() {
        let result = build_tree(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_ツリー構築_ルート直下のファイル() {
        let entries = vec![entry("README.md", "modified")];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "README.md");
        assert_eq!(tree[0].node_type, "file");
        assert_eq!(tree[0].status.as_deref(), Some("modified"));
        assert_eq!(tree[0].path, "README.md");
    }

    #[test]
    fn test_ツリー構築_単一子ディレクトリの折り畳み() {
        let entries = vec![entry("src/components/panels/Review.tsx", "new")];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "src/components/panels");
        assert_eq!(tree[0].node_type, "folder");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].name, "Review.tsx");
        assert_eq!(tree[0].children[0].node_type, "file");
    }

    #[test]
    fn test_ツリー構築_同一ディレクトリ複数ファイル() {
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
    fn test_ツリー構築_複数子は折り畳まない() {
        let entries = vec![
            entry("src/a.ts", "modified"),
            entry("src/b.ts", "deleted"),
            entry("lib/c.ts", "new"),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 2); // lib, src
        for node in &tree {
            assert_eq!(node.node_type, "folder");
        }
    }

    #[test]
    fn test_ツリー構築_深いネスト() {
        let entries = vec![
            entry("a/b/c/d/e.txt", "modified"),
            entry("a/b/c/d/f.txt", "new"),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "a/b/c/d");
        assert_eq!(tree[0].node_type, "folder");
        assert_eq!(tree[0].children.len(), 2);
    }

    #[test]
    fn test_ツリー構築_混在する深さ() {
        let entries = vec![
            entry("Cargo.toml", "modified"),
            entry("src/main.rs", "modified"),
            entry("src/git/diff.rs", "new"),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 2);

        let cargo = tree.iter().find(|n| n.name == "Cargo.toml").unwrap();
        assert_eq!(cargo.node_type, "file");

        let src = tree.iter().find(|n| n.name == "src").unwrap();
        assert_eq!(src.node_type, "folder");
        assert_eq!(src.children.len(), 2);
    }

    #[test]
    fn test_ツリー構築_統計がノードへ伝播() {
        let entries = vec![entry_with_stats("file.rs", "modified", 10, 3)];
        let tree = build_tree(entries);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].additions, Some(10));
        assert_eq!(tree[0].deletions, Some(3));
    }

    #[test]
    fn test_ツリー構築_フォルダは統計を持たない() {
        let entries = vec![
            entry_with_stats("src/a.ts", "modified", 5, 2),
            entry_with_stats("src/b.ts", "new", 20, 0),
        ];
        let tree = build_tree(entries);
        assert_eq!(tree[0].node_type, "folder");
        assert_eq!(tree[0].additions, None);
        assert_eq!(tree[0].deletions, None);
        assert_eq!(tree[0].children[0].additions, Some(5));
        assert_eq!(tree[0].children[1].additions, Some(20));
    }

    #[test]
    fn test_ナビゲーション_空ツリー() {
        let tree = build_tree(vec![]);
        let result = get_file_navigation(&tree, "anything.rs");
        assert_eq!(result.total, 0);
        assert_eq!(result.current_index, 0);
        assert!(result.prev_file.is_none());
        assert!(result.next_file.is_none());
    }

    #[test]
    fn test_ナビゲーション_単一ファイル() {
        let tree = build_tree(vec![entry("README.md", "modified")]);
        let result = get_file_navigation(&tree, "README.md");
        assert_eq!(result.total, 1);
        assert_eq!(result.current_index, 0);
        assert!(result.prev_file.is_none());
        assert!(result.next_file.is_none());
    }

    #[test]
    fn test_ナビゲーション_先頭ファイルはprevなし() {
        let tree = build_tree(vec![
            entry("a.rs", "modified"),
            entry("b.rs", "modified"),
            entry("c.rs", "modified"),
        ]);
        let result = get_file_navigation(&tree, "a.rs");
        assert_eq!(result.current_index, 0);
        assert_eq!(result.total, 3);
        assert!(result.prev_file.is_none());
        assert_eq!(result.next_file.as_deref(), Some("b.rs"));
    }

    #[test]
    fn test_ナビゲーション_末尾ファイルはnextなし() {
        let tree = build_tree(vec![
            entry("a.rs", "modified"),
            entry("b.rs", "modified"),
            entry("c.rs", "modified"),
        ]);
        let result = get_file_navigation(&tree, "c.rs");
        assert_eq!(result.current_index, 2);
        assert_eq!(result.total, 3);
        assert_eq!(result.prev_file.as_deref(), Some("b.rs"));
        assert!(result.next_file.is_none());
    }

    #[test]
    fn test_ナビゲーション_中間ファイル() {
        let tree = build_tree(vec![
            entry("a.rs", "modified"),
            entry("b.rs", "new"),
            entry("c.rs", "deleted"),
        ]);
        let result = get_file_navigation(&tree, "b.rs");
        assert_eq!(result.current_index, 1);
        assert_eq!(result.total, 3);
        assert_eq!(result.prev_file.as_deref(), Some("a.rs"));
        assert_eq!(result.next_file.as_deref(), Some("c.rs"));
    }

    #[test]
    fn test_ナビゲーション_ネストフォルダ() {
        let tree = build_tree(vec![
            entry("src/a.ts", "modified"),
            entry("src/b.ts", "new"),
            entry("lib/c.ts", "modified"),
        ]);
        // BTreeMap sorts: lib < src, so order is lib/c.ts, src/a.ts, src/b.ts
        let result = get_file_navigation(&tree, "src/a.ts");
        assert_eq!(result.total, 3);
        assert_eq!(result.current_index, 1);
        assert_eq!(result.prev_file.as_deref(), Some("lib/c.ts"));
        assert_eq!(result.next_file.as_deref(), Some("src/b.ts"));
    }

    #[test]
    fn test_ナビゲーション_現在ファイルが見つからない() {
        let tree = build_tree(vec![entry("a.rs", "modified"), entry("b.rs", "new")]);
        let result = get_file_navigation(&tree, "nonexistent.rs");
        assert_eq!(result.total, 2);
        assert_eq!(result.current_index, 0);
        assert!(result.prev_file.is_none());
        assert!(result.next_file.is_none());
    }

    #[test]
    fn test_ナビゲーション_結合ツリーで重複排除() {
        let staged = build_tree(vec![entry("a.rs", "modified"), entry("b.rs", "modified")]);
        let changes = build_tree(vec![entry("b.rs", "modified"), entry("c.rs", "new")]);
        let combined: Vec<_> = staged.into_iter().chain(changes).collect();

        let result = get_file_navigation(&combined, "b.rs");
        assert_eq!(result.total, 3); // a.rs, b.rs, c.rs (not 4)
        assert_eq!(result.current_index, 1);
        assert_eq!(result.prev_file.as_deref(), Some("a.rs"));
        assert_eq!(result.next_file.as_deref(), Some("c.rs"));
    }

    #[test]
    fn test_ツリー構築_ファイルとディレクトリ置換を両方保持() {
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
