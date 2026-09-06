use super::*;

fn load_unconsumed_source(source: &str) -> Result<LuaWorkflowDefinition, LuaWorkflowError> {
    let directory = tempfile::tempdir().unwrap();
    load_lua_workflow(
        "review.lua",
        source,
        directory.path(),
        LuaFacetCatalog::default(),
    )
}

#[test]
fn test_lua未消費参照_sequenceのchildの予約fieldを受理する() {
    // Given
    let source = r#"local r = require('releash')
local c = r.command{ name = 'check', command = 'true' }
local s = r.sequence{ children = { r.child{ node = c } } }
local ref = s.check.ok
return r.workflow{ name = 'example', description = 'example', main = s }
"#;

    // When
    let loaded = load_unconsumed_source(source).unwrap();

    // Then
    crate::domain::workflow::services::validation::validate(&loaded.workflow).unwrap();
}

#[test]
fn test_lua未消費参照_入れ子のsequenceとartifactの多段fieldを受理する() {
    // Given
    let source = r#"local r = require('releash')
local result = r.schema.object{ properties = {
  nested = r.schema.object{ properties = { passed = r.schema.boolean() } },
} }
local c = r.command{ name = 'check', command = 'true', artifact = result }
local s = r.sequence{ name = 'part', children = { r.child{ node = c } } }
local main = r.sequence{ children = { r.child{ node = s } } }
local ref = main.part.check.nested.passed
return r.workflow{ name = 'example', description = 'example', main = main }
"#;

    // When
    let loaded = load_unconsumed_source(source).unwrap();

    // Then
    crate::domain::workflow::services::validation::validate(&loaded.workflow).unwrap();
}

#[test]
fn test_lua未消費参照_sequenceの未知のchildとfieldを添字アクセス行で拒否する() {
    // Given
    for path in ["s.missing.ok", "s.check.missing", "s.check.ok.missing"] {
        let source = format!(
            r#"local r = require('releash')
local c = r.command{{ name = 'check', command = 'true' }}
local s = r.sequence{{ children = {{ r.child{{ node = c }} }} }}
local ref = {path}
return r.workflow{{ name = 'example', description = 'example', main = s }}
"#
        );

        // When
        let error = load_unconsumed_source(&source).unwrap_err();

        // Then
        assert_eq!(error.code, "WFR003", "{path}");
        assert_eq!(
            error.location,
            Some(LuaSourceLocation {
                source: "review.lua".to_string(),
                line: 4,
            })
        );
    }
}

#[test]
fn test_lua未消費参照_mainから到達しないdraftは検証しない() {
    // Given
    let source = r#"local r = require('releash')
local draft = r.command{ name = 'draft', command = 'true' }
local ref = draft.missing
return r.workflow{ name = 'example', description = 'example', main = r.command{ command = 'true' } }
"#;

    // When
    let loaded = load_unconsumed_source(source).unwrap();

    // Then
    assert_eq!(loaded.workflow.nodes.len(), 1);
    crate::domain::workflow::services::validation::validate(&loaded.workflow).unwrap();
}

#[test]
fn test_lua未消費参照_fanoutのchildの未配線参照を従来どおり受理する() {
    // Given
    let source = r#"local r = require('releash')
local c = r.command{ name = 'check', command = 'true' }
local ref = c.ok
return r.workflow{ name = 'example', description = 'example', main = r.fanout{ children = { r.child{ node = c } } } }
"#;

    // When
    let loaded = load_unconsumed_source(source).unwrap();

    // Then
    crate::domain::workflow::services::validation::validate(&loaded.workflow).unwrap();
}

#[test]
fn test_lua未消費参照_inputのcontractを従来どおり検証する() {
    // Given
    for (contract, field, accepted) in [
        (
            ", r.schema.object{ properties = { valid = r.schema.boolean() } }",
            "valid",
            true,
        ),
        (
            ", r.schema.object{ properties = { valid = r.schema.boolean() } }",
            "missing",
            false,
        ),
        ("", "missing", true),
    ] {
        let source = format!(
            r#"local r = require('releash')
local input = r.input('data'{contract})
local ref = input.{field}
return r.workflow{{ name = 'example', description = 'example', main = r.command{{ command = 'true' }} }}
"#
        );

        // When
        let result = load_unconsumed_source(&source);

        // Then
        if accepted {
            assert!(result.is_ok(), "{result:?}");
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "WFR003");
            assert_eq!(error.location.unwrap().line, 3);
        }
    }
}

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
