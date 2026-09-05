use super::*;

#[test]
fn test_lua参照パス_分岐とルートを正しい段順に復元する() {
    let mut paths = SourcePaths::new();
    let node = paths.root(SourceRoot::Node(0));
    let input = paths.root(SourceRoot::Input(0));
    let prefix = paths.child(node, "outer");
    let left = paths.child(prefix, "left");
    let right = paths.child(prefix, "right");
    let input_field = paths.child(input, "outer");

    assert_eq!(paths.child(prefix, "left"), left);
    assert_eq!(paths.fields(left), ["outer", "left"]);
    assert_eq!(paths.fields(right), ["outer", "right"]);
    assert_eq!(paths.fields(input_field), ["outer"]);
    for root in [
        node,
        input,
        SourceDraft::Request.path(),
        SourceDraft::Items.path(),
    ] {
        assert!(paths.is_root(root));
        assert!(paths.fields(root).is_empty());
    }
    assert_eq!(SourceDraft::Request.path(), SourcePaths::REQUEST);
    assert_eq!(SourceDraft::Items.path(), SourcePaths::ITEMS);
}

#[test]
fn test_lua参照パス_共有しても参照ごとの位置を保持する() {
    for kind in [HANDLE_NODE, HANDLE_INPUT] {
        let mut host = WorkflowLuaHost::new(LuaFacetCatalog::default());
        let root = LuaHostHandle {
            kind: kind.to_string(),
            index: 0,
        };
        for line in [3, 7] {
            host.index(
                &root,
                "field",
                LuaSourceLocation {
                    source: "review.lua".to_string(),
                    line,
                },
            )
            .unwrap();
        }
        let first = &host.sources[2];
        let second = &host.sources[3];
        assert_eq!(first.path(), second.path());
        for (source, line) in [(first, 3), (second, 7)] {
            let location = match source {
                SourceDraft::Node { location, .. } | SourceDraft::Input { location, .. } => {
                    location
                }
                _ => panic!("expected indexed source"),
            };
            assert_eq!(location.line, line);
            assert_eq!(location.source, "review.lua");
        }
    }
}

#[test]
fn test_lua参照パス_深いチェーンを線形に保持してarena上限で拒否する() {
    for kind in [HANDLE_NODE, HANDLE_INPUT] {
        let mut host = WorkflowLuaHost::new(LuaFacetCatalog::default());
        let mut current = LuaHostHandle {
            kind: kind.to_string(),
            index: 0,
        };
        let location = LuaSourceLocation {
            source: "review.lua".to_string(),
            line: 4,
        };
        let depth = MAX_HOST_ARENA_ENTRIES - host.arena_entries();
        for _ in 0..depth {
            let LuaData::Handle(next) = host.index(&current, "field", location.clone()).unwrap()
            else {
                panic!("expected source handle");
            };
            current = next;
        }
        let path = host.sources[current.index].path();
        assert_eq!(host.source_paths.parents.len(), depth + 3);
        assert_eq!(host.source_paths.fields(path), vec!["field"; depth]);
        host.mark_source_consumed(current.index);
        assert!(host
            .sources
            .iter()
            .skip(2)
            .all(|source| host.source_paths.contains(source.path())));
        let error = host.index(&current, "field", location.clone()).unwrap_err();
        assert_eq!(error.category, "WFS010");
        assert_eq!(error.location, Some(location));
        assert_eq!(host.arena_entries(), MAX_HOST_ARENA_ENTRIES);
        assert_eq!(host.source_paths.parents.len(), depth + 3);
    }
}
