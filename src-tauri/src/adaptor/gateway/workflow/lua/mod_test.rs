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

#[test]
fn test_lua容量検査_一件と述語サイズの追加は上限ちょうどまで受理する() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 7,
    };
    for additional in [1, 7] {
        for (total, accepted) in [(99_999, true), (100_000, true), (100_001, false)] {
            let mut host = WorkflowLuaHost::new(LuaFacetCatalog::default());
            let current = total - additional;
            host.predicate_entries = current - host.arena_entries();
            // When
            let result = host.ensure_arena_budget(additional, &location);
            // Then
            assert_eq!(host.arena_entries(), current);
            if accepted {
                assert!(result.is_ok(), "{current} + {additional}: {result:?}");
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.category, "WFS010");
                assert_eq!(
                    error.message,
                    "Lua definition exceeded the limit of 100000 builder values"
                );
                assert_eq!(error.field, None);
                assert_eq!(error.location, Some(location.clone()));
            }
        }
    }
}

#[test]
fn test_lua容量検査_追加件数の算術上限を超えても拒否する() {
    // Given
    let host = WorkflowLuaHost::new(LuaFacetCatalog::default());
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 3,
    };
    // When
    let error = host.ensure_arena_budget(usize::MAX, &location).unwrap_err();
    // Then
    assert_eq!(error.category, "WFS010");
    assert_eq!(
        error.message,
        "Lua definition exceeded the limit of 100000 builder values"
    );
    assert_eq!(error.field, None);
    assert_eq!(error.location, Some(location));
}

#[test]
fn test_lua容量検査_空述語と入口の容量エラーの優先順位を保つ() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 3,
    };
    for function in [FN_ALL, FN_ANY] {
        for (current, code, message) in [
            (
                99_999,
                "WFS002",
                "predicate and/or must contain at least one element",
            ),
            (
                100_000,
                "WFS010",
                "Lua definition exceeded the limit of 100000 builder values",
            ),
        ] {
            let mut host = WorkflowLuaHost::new(LuaFacetCatalog::default());
            host.predicate_entries = current - host.arena_entries();
            // When
            let error = host
                .call(
                    function,
                    vec![LuaData::Table(LuaTableData::default())],
                    location.clone(),
                )
                .unwrap_err();
            // Then
            assert_eq!(error.category, code);
            assert_eq!(error.message, message);
            assert_eq!(error.location, Some(location.clone()));
            assert_eq!(error.field, None);
            assert_eq!(host.arena_entries(), current);
            assert!(host.predicates.is_empty());
        }
    }
}

#[test]
fn source_pathsは供給元と段が同じ別sourceをprefixまで消費済みにする() {
    // Given
    let mut paths = SourcePaths::new();
    let root = paths.root(SourceRoot::Node(3));
    let same_prefix = paths.child(root, "a");
    let same_prefix_from_another_reference = paths.child(root, "a");
    let consumed_source = paths.child(same_prefix, "b");
    let sibling = paths.child(same_prefix, "c");

    // When
    paths.mark(consumed_source);

    // Then
    assert!(paths.contains(consumed_source));
    assert!(paths.contains(same_prefix));
    assert!(paths.contains(same_prefix_from_another_reference));
    assert!(!paths.contains(sibling));
}

#[test]
fn source_pathsは深いsourceの各段を線形個のpathとして追跡する() {
    // Given
    const DEPTH: usize = 10_000;
    let mut paths = SourcePaths::new();
    let mut path = paths.root(SourceRoot::Input(7));
    let mut prefixes = Vec::with_capacity(DEPTH);
    for index in 0..DEPTH {
        path = paths.child(path, &format!("field{index}"));
        prefixes.push(path);
    }

    // When
    paths.mark(path);

    // Then
    assert_eq!(paths.parents.len(), DEPTH + 3);
    assert!(prefixes.into_iter().all(|path| paths.contains(path)));
}

mod restored_memory_tests {
    use super::super::*;

    const DELEGATE_YAML: &str = r#"name: delegate
description: test
schemas:
  result:
    type: object
    properties: {done: {type: boolean}, task: {type: string}}
    required: [done, task]
  verdict:
    type: object
    properties:
      complete: {type: boolean}
      detail: {type: object, properties: {clean: {type: boolean}}, required: [clean]}
    required: [complete, detail]
nodes:
  main:
    sequence:
      children:
        - work: {inputs: {task: request}}
  work:
    session: {provider: codex, facets: {instruction: implement_fix_plan}}
    artifact: result
    input: [task]
    completion:
      require: approval
      delegate: {child: check, inputs: {result: work}, when: child.complete, max_iterations: 3}
  check:
    session: {provider: codex, facets: {instruction: implement_fix_plan}}
    artifact: verdict
    input: [result]
"#;

    #[test]
    pub fn test_completion_delegate_生成名参照と内部snapshot表記をyamlで受理しない() {
        // Given
        for (source, code, message) in [
            (
                "main#0",
                "WFR007",
                "source must be `<name>` or `<name>.<field>...`",
            ),
            (
                "{node_artifact: main#0}",
                "WFS002",
                "invalid type: map, expected a string",
            ),
        ] {
            let yaml = DELEGATE_YAML.replace(
                "inputs: {result: work}",
                &format!("inputs: {{result: {source}}}"),
            );
            // When
            let result = crate::adaptor::gateway::workflow::diagnostics::diagnose_workflow_source(
                &yaml, None,
            );
            // Then
            assert!(result.has_errors());
            assert_eq!(result.diagnostics.len(), 1, "{:?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].code, code);
            assert!(
                result.diagnostics[0].message.contains(message),
                "{:?}",
                result.diagnostics
            );
        }
    }

    #[test]
    pub fn test_completion_delegate_yamlのinputs記述順を保存復元前後のread_modelが保持する() {
        // Given
        let yaml = DELEGATE_YAML
            .replace("input: [result]", "input: [zebra, apple, middle]")
            .replace(
                "inputs: {result: work}",
                "inputs: {zebra: work, apple: work.task, middle: request}",
            );
        // When
        let loaded =
            crate::adaptor::gateway::workflow::diagnostics::diagnose_workflow_source(&yaml, None);
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        let workflow = loaded.workflow.unwrap();
        let restored: WorkflowDefinition =
            serde_json::from_str(&serde_json::to_string(&workflow).unwrap()).unwrap();
        // Then
        for definition in [&workflow, &restored] {
            let dto = crate::usecase::workflow::dto::workflow_to_dto(definition);
            let work = dto.nodes.iter().find(|node| node.name == "work").unwrap();
            let delegate = work.completion.as_ref().unwrap().delegate.as_ref().unwrap();
            assert_eq!(
                delegate
                    .inputs
                    .iter()
                    .map(|input| (input.parameter.as_str(), input.source.as_str()))
                    .collect::<Vec<_>>(),
                [
                    ("zebra", "work"),
                    ("apple", "work.task"),
                    ("middle", "request"),
                ]
            );
        }
    }
}
