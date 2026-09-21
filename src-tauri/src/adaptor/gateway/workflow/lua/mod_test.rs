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

#[test]
fn test_lua未消費参照_fanoutの名前キーと添字キーと入れ子を受理する() {
    // Given
    for (items, path) in [
        ("", "main.fan.check.ok"),
        ("items = {'one'},", "main.fan['0'].ok"),
        ("items = {'one'},", "main.fan['100'].ok"),
        ("", "main.fan"),
    ] {
        let source = format!(
            r#"local r = require('releash')
local c = r.command{{ name = 'check', command = 'true', input = {{ r.input('item') }} }}
local fan = r.fanout{{ name = 'fan', {items} children = {{ r.child{{ node = c }} }} }}
local main = r.sequence{{ children = {{ r.child{{ node = fan }} }} }}
local ref = {path}
return r.workflow{{ name = 'example', description = 'example', main = main }}
"#
        );

        // When
        let loaded = load_unconsumed_source(&source);

        // Then
        assert!(loaded.is_ok(), "{path}: {loaded:?}");
    }
}

#[test]
fn test_lua未消費参照_fanoutの未知キーと非正準添字と非objectを拒否する() {
    // Given
    for (items, path, message) in [
        (
            "",
            "fan.missing.ok",
            "artifact field 'missing' does not exist",
        ),
        (
            "items = {'one'},",
            "fan['007'].ok",
            "artifact field '007' does not exist",
        ),
        (
            "",
            "fan.check.ok.missing",
            "artifact field 'missing' cannot be read from a non-object schema",
        ),
    ] {
        let source = format!(
            r#"local r = require('releash')
local c = r.command{{ name = 'check', command = 'true' }}
local fan = r.fanout{{ {items} children = {{ r.child{{ node = c }} }} }}
local ref = {path}
return r.workflow{{ name = 'example', description = 'example', main = fan }}
"#
        );

        // When
        let error = load_unconsumed_source(&source).unwrap_err();

        // Then
        assert_eq!(error.code, "WFR003");
        assert_eq!(error.message, message);
        assert_eq!(error.location.unwrap().line, 4);
    }
}

const PREDICATE_SOURCE: &str = include_str!("../fixtures/valid/predicate-routing.lua");
const PREDICATE_EXPRESSION: &str = "r.all{ judge.passed, r.any{ judge.clean, judge.skipped } }";

#[test]
fn test_lua参照解決_三つの利用箇所で参照起点と位置と容量計上を保持する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for value in ["source", "producer", "input"] {
        for (builder, expression) in [
            (
                "child",
                format!("r.child{{ node = producer, inputs = {{ data = {value} }} }}"),
            ),
            (
                "switch",
                format!("r.switch{{ on = {value}, cases = {{ yes = producer }} }}"),
            ),
            (
                "when",
                format!("r.when{{ on = {value}, on_true = producer, next = producer }}"),
            ),
        ] {
            let source = format!(
                r#"local r = require('releash')
local producer = r.command{{ command = 'true' }}
local input = r.input('data')
local source = producer.passed
return {expression}
"#
            );
            // When
            let evaluation = evaluate(
                LuaEvaluationRequest {
                    source_name: "sources.lua",
                    source: &source,
                    workflows_dir: directory.path(),
                    limits: LuaLimits::default(),
                },
                WorkflowLuaHost::new(LuaFacetCatalog::default()),
            )
            .unwrap();
            // Then
            let host = evaluation.host;
            let index = match builder {
                "child" => {
                    assert_eq!(evaluation.value, handle(HANDLE_CHILD, 0));
                    assert_eq!(host.children[0].inputs.len(), 1);
                    assert_eq!(host.children[0].inputs[0].0, "data");
                    host.children[0].inputs[0].1
                }
                _ => {
                    assert_eq!(evaluation.value, handle(HANDLE_RULE, 0));
                    match &host.rules[0] {
                        RuleDraft::Switch { on, .. } => *on,
                        RuleDraft::When {
                            on: Predicate::Ref(index),
                            ..
                        } => *index,
                        other => panic!("{other:?}"),
                    }
                }
            };
            let added_sources = usize::from(value != "source");
            assert_eq!(index, if value == "source" { 2 } else { 3 });
            assert_eq!(host.sources.len(), 3 + added_sources);
            assert_eq!(host.predicate_entries, 0);
            assert_eq!(host.arena_entries(), 6 + added_sources);
            let (root, path, location) = match &host.sources[index] {
                SourceDraft::Node {
                    node,
                    path,
                    location,
                } => (SourceRoot::Node(*node), path, location),
                SourceDraft::Input {
                    input,
                    path,
                    location,
                } => (SourceRoot::Input(*input), path, location),
                other => panic!("{other:?}"),
            };
            assert_eq!(
                root,
                if value == "input" {
                    SourceRoot::Input(0)
                } else {
                    SourceRoot::Node(0)
                }
            );
            let expected_fields: Vec<&str> = if value == "source" {
                vec!["passed"]
            } else {
                vec![]
            };
            assert_eq!(host.source_paths.fields(*path), expected_fields);
            assert_eq!(location.source, "sources.lua");
            assert_eq!(location.line, if value == "source" { 4 } else { 5 });
        }
    }
}

fn predicate_budget_host(expression: &str) -> (WorkflowLuaHost, LuaData) {
    let directory = tempfile::tempdir().unwrap();
    let source = format!(
        "local r = require('releash')\n\
         local judge = r.command{{ command = 'judge' }}\n\
         local done = r.command{{ command = 'done' }}\n\
         local fix = r.command{{ command = 'fix' }}\n\
         local source = judge.passed\n\
         return {expression}\n"
    );
    let evaluation = evaluate(
        LuaEvaluationRequest {
            source_name: "budget.lua",
            source: &source,
            workflows_dir: directory.path(),
            limits: LuaLimits::default(),
        },
        WorkflowLuaHost::new(LuaFacetCatalog::default()),
    )
    .unwrap();
    (evaluation.host, evaluation.value)
}

fn when_arguments(on: LuaData) -> Vec<LuaData> {
    vec![LuaData::Table(LuaTableData {
        entries: [
            ("on", on),
            ("on_true", handle(HANDLE_NODE, 1)),
            ("next", handle(HANDLE_NODE, 2)),
        ]
        .into_iter()
        .map(|(key, value)| (LuaTableKey::String(key.to_string()), value))
        .collect(),
    })]
}

#[test]
fn test_lua述語解決_存在しないhandleは利用箇所のfieldと位置で拒否する() {
    // Given
    let location = LuaSourceLocation {
        source: "missing-predicate.lua".to_string(),
        line: 7,
    };
    for function in [FN_WHEN, FN_ALL, FN_ANY] {
        let (mut host, _) = predicate_budget_host("source");
        let before = host.arena_entries();
        let value = handle(HANDLE_PREDICATE, host.predicates.len());
        let (arguments, field) = if function == FN_WHEN {
            (when_arguments(value), "on")
        } else {
            (
                vec![LuaData::Table(LuaTableData {
                    entries: [(LuaTableKey::Integer(1), value)].into_iter().collect(),
                })],
                "predicate element",
            )
        };

        // When
        let error = host
            .call(function, arguments, location.clone())
            .unwrap_err();

        // Then
        assert_eq!(error.category, "WFS002");
        assert_eq!(
            error.message,
            "predicate must be a field reference or an and/or map"
        );
        assert_eq!(error.field.as_deref(), Some(field));
        assert_eq!(error.location, Some(location.clone()));
        assert_eq!(host.arena_entries(), before);
        assert_eq!(host.predicate_entries, 0);
        assert!(host.predicates.is_empty());
        assert!(host.rules.is_empty());
    }
}

#[test]
fn test_lua単一参照のwhen_rule一件だけ計上してbaseの容量境界を保つ() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 7,
    };
    for current in [None, Some(99_999), Some(100_000)] {
        let (mut host, source) = predicate_budget_host("source");
        let source_index = expect_handle(&source, HANDLE_SOURCE).unwrap();
        if let Some(current) = current {
            host.predicate_entries = current - host.arena_entries();
        }
        let before = host.arena_entries();
        let predicate_entries = host.predicate_entries;
        let sources = host.sources.len();

        // When
        let result = host.call(FN_WHEN, when_arguments(source), location.clone());

        // Then
        assert_eq!(host.sources.len(), sources);
        assert!(host.predicates.is_empty());
        assert_eq!(host.predicate_entries, predicate_entries);
        if before < MAX_HOST_ARENA_ENTRIES {
            assert_eq!(result.unwrap(), handle(HANDLE_RULE, 0));
            assert_eq!(host.arena_entries(), before + 1);
            assert_eq!(host.rules.len(), 1);
            let RuleDraft::When { on, on_true, next } = &host.rules[0] else {
                panic!("expected when rule");
            };
            assert_eq!(*on, Predicate::Ref(source_index));
            assert_eq!((*on_true, *next), (1, 2));
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.category, "WFS010");
            assert_eq!(
                error.message,
                "Lua definition exceeded the limit of 100000 builder values"
            );
            assert_eq!(error.field, None);
            assert_eq!(error.location, Some(location.clone()));
            assert_eq!(host.arena_entries(), before);
            assert!(host.rules.is_empty());
        }
    }
}

#[test]
fn test_lua合成述語_同じsourceの各refと根を計上して上限で拒否する() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 7,
    };
    for function in [FN_ALL, FN_ANY] {
        for count in [1, 3] {
            for remaining in [None, Some(count + 1), Some(count)] {
                let (mut host, source) = predicate_budget_host("source");
                let source_index = expect_handle(&source, HANDLE_SOURCE).unwrap();
                if let Some(remaining) = remaining {
                    host.predicate_entries =
                        MAX_HOST_ARENA_ENTRIES - remaining - host.arena_entries();
                }
                let before = host.arena_entries();
                let predicate_entries = host.predicate_entries;
                let sources = host.sources.len();
                let arguments = vec![LuaData::Table(LuaTableData {
                    entries: (1..=count)
                        .map(|index| (LuaTableKey::Integer(index as i64), source.clone()))
                        .collect(),
                })];

                // When
                let result = host.call(function, arguments, location.clone());

                // Then
                assert_eq!(host.sources.len(), sources);
                assert!(host.rules.is_empty());
                if remaining != Some(count) {
                    assert_eq!(result.unwrap(), handle(HANDLE_PREDICATE, 0));
                    assert_eq!(host.arena_entries(), before + count + 1);
                    assert_eq!(host.predicate_entries, predicate_entries + count + 1);
                    let refs = vec![Predicate::Ref(source_index); count];
                    let expected = if function == FN_ALL {
                        Predicate::And(refs)
                    } else {
                        Predicate::Or(refs)
                    };
                    assert_eq!(host.predicates, vec![(expected, count + 1)]);
                } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.category, "WFS010");
                    assert_eq!(
                        error.message,
                        "Lua definition exceeded the limit of 100000 builder values"
                    );
                    assert_eq!(error.field, None);
                    assert_eq!(error.location, Some(location.clone()));
                    assert_eq!(host.arena_entries(), MAX_HOST_ARENA_ENTRIES);
                    assert!(host.predicates.is_empty());
                }
            }
        }
    }
}

#[test]
fn test_lua述語再利用_合成とwhenは複製全体と一件を計上して上限で拒否する() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 7,
    };
    for function in [FN_ALL, FN_ANY, FN_WHEN] {
        for remaining in [None, Some(6), Some(5), Some(4)] {
            let (mut host, value) =
                predicate_budget_host("r.all{ source, r.any{ source, source } }");
            let (predicate, size) = host.predicates[1].clone();
            assert_eq!(size, 5);
            assert_eq!(host.predicate_entries, 8);
            if let Some(remaining) = remaining {
                host.predicate_entries += MAX_HOST_ARENA_ENTRIES - remaining - host.arena_entries();
            }
            let before = host.arena_entries();
            let predicate_entries = host.predicate_entries;
            let sources = host.sources.len();
            let arguments = if function == FN_WHEN {
                when_arguments(value)
            } else {
                vec![LuaData::Table(LuaTableData {
                    entries: [(LuaTableKey::Integer(1), value)].into_iter().collect(),
                })]
            };

            // When
            let result = host.call(function, arguments, location.clone());

            // Then
            assert_eq!(host.sources.len(), sources);
            if remaining.is_none_or(|remaining| remaining > size) {
                assert_eq!(host.arena_entries(), before + size + 1);
                if function == FN_WHEN {
                    assert_eq!(result.unwrap(), handle(HANDLE_RULE, 0));
                    assert_eq!(host.predicate_entries, predicate_entries + size);
                    assert_eq!(host.predicates.len(), 2);
                    assert_eq!(host.rules.len(), 1);
                    let RuleDraft::When { on, on_true, next } = &host.rules[0] else {
                        panic!("expected when rule");
                    };
                    assert_eq!(*on, predicate);
                    assert_eq!((*on_true, *next), (1, 2));
                } else {
                    assert_eq!(result.unwrap(), handle(HANDLE_PREDICATE, 2));
                    assert_eq!(host.predicate_entries, predicate_entries + size + 1);
                    assert!(host.rules.is_empty());
                    let expected = if function == FN_ALL {
                        Predicate::And(vec![predicate])
                    } else {
                        Predicate::Or(vec![predicate])
                    };
                    assert_eq!(host.predicates[2], (expected, size + 1));
                }
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.category, "WFS010");
                assert_eq!(
                    error.message,
                    "Lua definition exceeded the limit of 100000 builder values"
                );
                assert_eq!(error.field, None);
                assert_eq!(error.location, Some(location.clone()));
                let added = if remaining == Some(size) { size } else { 0 };
                assert_eq!(host.arena_entries(), before + added);
                assert_eq!(host.predicate_entries, predicate_entries + added);
                assert!(host.rules.is_empty());
                assert_eq!(host.predicates.len(), 2);
            }
        }
    }
}

#[test]
fn test_lua容量検査_一件と述語サイズの追加は上限ちょうどまで受理する() {
    // Given
    assert_eq!(MAX_HOST_ARENA_ENTRIES, 100_000);
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
fn test_lua述語_不正な要素と自child以外の参照起点を拒否する() {
    // Given
    for (expression, code) in [
        ("r.all{ true }", "WFS002"),
        ("r.any{ 'passed' }", "WFS002"),
        ("r.all{ 3 }", "WFS002"),
        ("r.any{ {} }", "WFS002"),
        ("r.all{ judge.passed, r.any{ false } }", "WFS002"),
        ("r.all{ field = judge.passed }", "WFS002"),
        ("r.all{ judge.passed, r.any{ done.ok } }", "WFR003"),
        ("r.any{ judge.passed, r.request }", "WFR003"),
        ("r.all{ judge.passed, r.items }", "WFR003"),
        ("r.any{ r.input('value') }", "WFR003"),
        ("r.any{ r.input('value', r.schema.object{properties = {passed = r.schema.boolean()}, required = {'passed'}}).passed }", "WFR003"),
        ("r.all{ judge }", "WFR003"),
    ] {
        let source = PREDICATE_SOURCE.replace(PREDICATE_EXPRESSION, expression);
        // When
        let error = load_unconsumed_source(&source).unwrap_err();
        // Then
        assert_eq!(error.code, code, "{expression}: {error:?}");
    }
}

#[test]
fn test_lua述語_switchのsourceには述語を受理しない() {
    // Given
    let source = PREDICATE_SOURCE.replace("r.when{", "r.switch{").replace(
        "on_true = done, next = fix",
        "cases = { yes = done }, next = fix",
    );
    // When
    let error = load_unconsumed_source(&source).unwrap_err();
    // Then
    assert_eq!(error.code, "WFS002");
    assert_eq!(error.message, "field 'on' must be Source");
}

#[test]
fn test_lua述語_再利用による展開もarena上限に数える() {
    // Given
    let source = r#"local r = require('releash')
local judge = r.command{ command = 'judge' }
local predicate = r.all{ judge.ok }
for i = 1, 30 do predicate = r.all{ predicate, predicate } end
return r.workflow{ name = 'budget', description = 'test', main = judge }
"#;
    // When
    let error = load_unconsumed_source(source).unwrap_err();
    // Then
    assert_eq!(error.code, "WFS010");
    assert!(error.message.contains("builder values"));
}

#[test]
fn test_completion要求_luaのrequire値は承認handleだけを受理する() {
    // Given
    for value in ["'approval'", "r.provider.claude"] {
        let source = format!("local r = require('releash')\nreturn r.workflow{{ name = 'completion', description = 'test', main = r.command{{ command = 'true', completion = {{ require = {value} }} }} }}");
        // When
        let error = load_unconsumed_source(&source).unwrap_err();
        // Then
        assert_eq!(error.code, "WFS002");
        assert_eq!(error.message, "completion require must be approval");
        assert_eq!(error.field.as_deref(), Some("completion"));
    }
}

const DELEGATE_LUA: &str = r#"local r = require('releash')
local f = require('facets')
local result = r.schema.object{ name = 'result', properties = {
  done = r.schema.boolean(), task = r.schema.string{},
}, required = { 'done', 'task' } }
local verdict = r.schema.object{ name = 'verdict', properties = {
  complete = r.schema.boolean(), detail = r.schema.object{ properties = {
    clean = r.schema.boolean(),
  }, required = { 'clean' } },
}, required = { 'complete', 'detail' } }
local task = r.input('task')
local check = r.session{ name = 'check', provider = r.provider.codex,
  facets = { instruction = f.instruction.implement_fix_plan }, artifact = verdict,
  input = { r.input('result') },
}
local work = r.session{ name = 'work', provider = r.provider.codex,
  facets = { instruction = f.instruction.implement_fix_plan }, artifact = result,
  input = { task }, completion = { require = r.completion.approval },
}
local main = r.sequence{ children = { r.child{ node = work, inputs = { task = r.request } } } }
work.delegate{ child = check, inputs = { result = work }, when = work.child.complete, max_iterations = 3 }
return r.workflow{ name = 'delegate', description = 'test', main = main }
"#;

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

fn diagnose_delegate_lua(source: &str) -> super::super::diagnostics::WorkflowSourceDiagnostics {
    let directory = tempfile::tempdir().unwrap();
    super::super::diagnostics::diagnose_lua_workflow_source(
        "delegate.lua",
        source,
        directory.path(),
        directory.path(),
        None,
    )
}

fn assert_delegate_equivalent(lua: &str, yaml: &str, accepted: bool) -> Option<WorkflowDefinition> {
    let lua = diagnose_delegate_lua(lua);
    let yaml = super::super::diagnostics::diagnose_workflow_source(yaml, None);
    let signature = |result: &super::super::diagnostics::WorkflowSourceDiagnostics| {
        let mut items = result
            .diagnostics
            .iter()
            .map(|item| {
                (
                    item.code.clone(),
                    format!("{:?}", item.stage),
                    item.message.clone(),
                )
            })
            .collect::<Vec<_>>();
        items.sort();
        items
    };
    assert_eq!(signature(&lua), signature(&yaml));
    assert_eq!(
        lua.diagnostics.is_empty(),
        accepted,
        "{:?}",
        lua.diagnostics
    );
    if accepted {
        let lua = lua.workflow.unwrap();
        let yaml = yaml.workflow.unwrap();
        assert_eq!(lua, yaml);
        Some(lua)
    } else {
        None
    }
}

#[test]
fn test_completion_delegate_例を受理し承認とchildを含むyamlと同一定義になる() {
    // Given / When
    let workflow = assert_delegate_equivalent(DELEGATE_LUA, DELEGATE_YAML, true).unwrap();
    // Then
    assert_eq!(workflow.nodes.len(), 3);
    let completion = &workflow.node_by_name("work").unwrap().completion;
    assert!(completion.requires_approval());
    assert_eq!(completion.delegate.as_ref().unwrap().child, "check");
}

#[test]
fn test_completion_delegate_inputs省略と全供給元と述語をyamlと同じ配線にする() {
    // Given
    for (lua_inputs, yaml_inputs) in [
        ("", ""),
        ("inputs = { result = task }, ", "inputs: {result: task}, "),
        ("inputs = { result = work }, ", "inputs: {result: work}, "),
        (
            "inputs = { result = work.task }, ",
            "inputs: {result: work.task}, ",
        ),
        (
            "inputs = { result = work.child.detail.clean }, ",
            "inputs: {result: work.child.detail.clean}, ",
        ),
        (
            "inputs = { result = r.request }, ",
            "inputs: {result: request}, ",
        ),
    ] {
        for (lua_when, yaml_when) in [
            ("work.done", "done"),
            ("work.child.complete", "child.complete"),
            (
                "r.all{ work.done, r.any{ work.child.complete, work.child.detail.clean } }",
                "{and: [done, {or: [child.complete, child.detail.clean]}]}",
            ),
        ] {
            let mut lua = DELEGATE_LUA
                .replace("inputs = { result = work }, ", lua_inputs)
                .replace("when = work.child.complete", &format!("when = {lua_when}"));
            let mut yaml = DELEGATE_YAML
                .replace("inputs: {result: work}, ", yaml_inputs)
                .replace("when: child.complete", &format!("when: {yaml_when}"));
            if lua_inputs.is_empty() {
                lua = lua.replace("input = { r.input('result') },", "");
                yaml = yaml.replace("    input: [result]\n", "");
            }
            // When / Then
            assert_delegate_equivalent(&lua, &yaml, true);
        }
    }
}

#[test]
fn test_completion_delegate_必須field未知キーと回数のdiagnosticがyamlと一致する() {
    // Given
    for (lua_field, yaml_field, lua_replacement, yaml_replacement) in [
        ("child = check, ", "child: check, ", "", ""),
        (
            "when = work.child.complete, ",
            "when: child.complete, ",
            "",
            "",
        ),
        (", max_iterations = 3", ", max_iterations: 3", "", ""),
        (
            "max_iterations = 3",
            "max_iterations: 3",
            "extra = true, max_iterations = 3",
            "extra: true, max_iterations: 3",
        ),
        (
            "max_iterations = 3",
            "max_iterations: 3",
            "max_iterations = 0",
            "max_iterations: 0",
        ),
        (
            "max_iterations = 3",
            "max_iterations: 3",
            "max_iterations = -1",
            "max_iterations: -1",
        ),
        (
            "max_iterations = 3",
            "max_iterations: 3",
            "max_iterations = 1.5",
            "max_iterations: 1.5",
        ),
        (
            "max_iterations = 3",
            "max_iterations: 3",
            "max_iterations = 4294967296",
            "max_iterations: 4294967296",
        ),
        (
            "max_iterations = 3",
            "max_iterations: 3",
            "max_iterations = '3'",
            "max_iterations: '3'",
        ),
    ] {
        let lua = DELEGATE_LUA.replace(lua_field, lua_replacement);
        let yaml = DELEGATE_YAML.replace(yaml_field, yaml_replacement);
        // When / Then
        assert_delegate_equivalent(&lua, &yaml, false);
    }
}

#[test]
fn test_completion_delegate_artifact欠落とchildの自己参照root共有をyamlと同じ診断にする() {
    // Given
    for case in [
        "artifact",
        "self",
        "root",
        "composite",
        "delegate",
        "later_composite",
    ] {
        let mut lua = DELEGATE_LUA.to_string();
        let mut yaml = DELEGATE_YAML.to_string();
        match case {
            "artifact" => {
                lua = lua.replace("artifact = result,", "");
                yaml = yaml.replace("    artifact: result\n", "");
            }
            "self" | "root" => {
                let child = if case == "self" { "work" } else { "main" };
                lua = lua.replace("child = check,", &format!("child = {child},"));
                yaml = yaml.replace("child: check,", &format!("child: {child},"));
                lua = lua.replace(
                    "node = work, inputs",
                    "node = check }, r.child{ node = work, inputs",
                );
                yaml = yaml.replace("        - work:", "        - check\n        - work:");
            }
            "composite" => {
                lua = lua.replace(
                    "node = work, inputs",
                    "node = check }, r.child{ node = work, inputs",
                );
                yaml = yaml.replace("        - work:", "        - check\n        - work:");
            }
            "delegate" => {
                lua = lua.replace("local main =", "local other = r.session{ name = 'other', provider = r.provider.codex, facets = {instruction = f.instruction.implement_fix_plan}, artifact = result }\nother.delegate{ child = check, when = other.child.complete, max_iterations = 3 }\nlocal main =")
                    .replace("task = r.request } } } }", "task = r.request } }, r.child{node = other} } }");
                yaml = yaml.replace(
                    "        - work: {inputs: {task: request}}",
                    "        - work: {inputs: {task: request}}\n        - other",
                );
                yaml.push_str("  other:\n    session: {provider: codex, facets: {instruction: implement_fix_plan}}\n    artifact: result\n    completion: {delegate: {child: check, when: child.complete, max_iterations: 3}}\n");
            }
            _ => {
                lua = lua.replace("local main =", "local later = r.sequence{ name = 'later', children = {r.child{node = check}} }\nlocal main =")
                    .replace("task = r.request } } } }", "task = r.request } }, r.child{node = later} } }");
                yaml = yaml.replace(
                    "        - work: {inputs: {task: request}}",
                    "        - work: {inputs: {task: request}}\n        - later",
                );
                yaml.push_str("  later: {sequence: {children: [check]}}\n");
            }
        }
        // When / Then
        assert_delegate_equivalent(&lua, &yaml, false);
    }
}

#[test]
fn test_completion_delegate_childの全kindと統合mapをyamlと同じcontractで検査する() {
    // Given
    for kind in ["session", "command", "sequence", "fanout", "items"] {
        let mut lua = DELEGATE_LUA.to_string();
        let mut yaml = DELEGATE_YAML.to_string();
        let path = match kind {
            "session" => "child.detail.clean",
            "command" => {
                lua = lua.replace("local check = r.session{ name = 'check', provider = r.provider.codex,\n  facets = { instruction = f.instruction.implement_fix_plan },", "local check = r.command{ name = 'check', command = 'check',");
                yaml = yaml.replace(
                    "  check:\n    session: {provider: codex, facets: {instruction: implement_fix_plan}}",
                    "  check:\n    command: check",
                );
                "child.ok"
            }
            _ => {
                let builder = if kind == "sequence" {
                    "sequence"
                } else {
                    "fanout"
                };
                let items = if kind == "items" {
                    "items = { 'a' }, "
                } else {
                    ""
                };
                lua = lua.replace("local main =", &format!("local checks = r.{builder}{{ name = 'checks', {items}children = {{ r.child{{node = check}} }}, input = {{r.input('result')}} }}\nlocal main ="))
                    .replace("child = check,", "child = checks,");
                yaml = yaml.replace("child: check,", "child: checks,");
                yaml = yaml.replace("  check:\n", &format!(
                    "  checks:\n    {builder}:\n      children: [check]\n{}    input: [result]\n  check:\n",
                    if kind == "items" {
                        "      items: [a]\n"
                    } else {
                        ""
                    }
                ));
                if kind == "items" {
                    "child.0.complete"
                } else {
                    "child.check.complete"
                }
            }
        };
        let lua_path = format!("work.{}", path.replace(".0.", "[\"0\"]."));
        lua = lua.replace("when = work.child.complete", &format!("when = {lua_path}"));
        yaml = yaml.replace("when: child.complete", &format!("when: {path}"));
        // When / Then
        assert_delegate_equivalent(&lua, &yaml, true);
        for invalid in ["missing", "detail", "detail.missing"] {
            assert_delegate_equivalent(
                &lua.replace(
                    &format!("when = {lua_path}"),
                    &format!("when = {lua_path}.{invalid}"),
                ),
                &yaml.replace(&format!("when: {path}"), &format!("when: {path}.{invalid}")),
                false,
            );
        }
    }
}

#[test]
fn test_completion_delegate_inputsとwhenの存在しないfieldと不適合型を拒否する() {
    // Given
    for path in ["missing", "complete.missing", "detail.missing"] {
        for inputs in [false, true] {
            let (lua, yaml) = if inputs {
                (
                    DELEGATE_LUA
                        .replace("result = work }", &format!("result = work.child.{path} }}")),
                    DELEGATE_YAML.replace("result: work}", &format!("result: work.child.{path}}}")),
                )
            } else {
                (
                    DELEGATE_LUA.replace(
                        "when = work.child.complete",
                        &format!("when = work.child.{path}"),
                    ),
                    DELEGATE_YAML.replace("when: child.complete", &format!("when: child.{path}")),
                )
            };
            // When / Then
            assert_delegate_equivalent(&lua, &yaml, false);
            let unnamed = diagnose_delegate_lua(&lua.replace("name = 'work', ", ""));
            assert!(unnamed.has_errors());
            assert_eq!(unnamed.diagnostics.len(), 1, "{:?}", unnamed.diagnostics);
            assert_eq!(
                unnamed.diagnostics[0].code,
                if inputs { "WFR007" } else { "WFT001" }
            );
        }
    }
    for (lua_change, yaml_change) in [
        ("complete = r.schema.string{}", "complete: {type: string}"),
        (
            "complete = r.schema.object{properties = {}}",
            "complete: {type: object, properties: {}}",
        ),
    ] {
        assert_delegate_equivalent(
            &DELEGATE_LUA.replace("complete = r.schema.boolean()", lua_change),
            &DELEGATE_YAML.replace("complete: {type: boolean}", yaml_change),
            false,
        );
    }
    assert_delegate_equivalent(
        &DELEGATE_LUA.replace(
            "required = { 'complete', 'detail' }",
            "required = { 'detail' }",
        ),
        &DELEGATE_YAML.replace("required: [complete, detail]", "required: [detail]"),
        false,
    );
}

#[test]
fn test_completion_delegate_二重宣言をshapeエラーにしてfield名よりメソッドを優先する() {
    // Given
    let declaration = "work.delegate{ child = check, inputs = { result = work }, when = work.child.complete, max_iterations = 3 }";
    let duplicated = DELEGATE_LUA.replace(declaration, &format!("{declaration}\n{declaration}"));
    // When
    let result = diagnose_delegate_lua(&duplicated);
    // Then
    assert_eq!(result.diagnostics.len(), 1);
    let error = &result.diagnostics[0];
    assert_eq!(error.code, "WFS002");
    assert_eq!(
        error.stage,
        crate::adaptor::protocol::workflow::DiagnosticStage::ParseShape
    );
    assert!(error.message.contains("same Session handle"));
    assert!(error.message.contains("twice"));
    // Given / When / Then
    assert_delegate_equivalent(
        &DELEGATE_LUA.replace(
            "done = r.schema.boolean(),",
            "delegate = r.schema.boolean(), done = r.schema.boolean(),",
        ),
        &DELEGATE_YAML.replace(
            "properties: {done:",
            "properties: {delegate: {type: boolean}, done:",
        ),
        true,
    );
}

#[test]
fn test_completion_delegate_同名fieldがあってもメソッドを供給元として受理しない() {
    // Given
    let source = DELEGATE_LUA.replace(
        "done = r.schema.boolean(),",
        "delegate = r.schema.boolean(), done = r.schema.boolean(),",
    );
    for (old, new) in [
        (
            "inputs = { result = work }",
            "inputs = { result = work.delegate }",
        ),
        ("when = work.child.complete", "when = work.delegate"),
        (
            "when = work.child.complete",
            "when = r.all{ work.done, work.delegate }",
        ),
        (
            "when = work.child.complete",
            "when = r.any{ work.done, work.delegate }",
        ),
    ] {
        // When
        let result = diagnose_delegate_lua(&source.replace(old, new));
        // Then
        assert!(result.workflow.is_none(), "{new}");
        assert!(result.has_errors(), "{new}");
        assert_eq!(result.diagnostics.len(), 1, "{new}");
        let error = &result.diagnostics[0];
        assert_eq!(error.code, "WFS010", "{new}");
        assert_eq!(
            error.stage,
            crate::adaptor::protocol::workflow::DiagnosticStage::ParseShape,
            "{new}"
        );
        assert!(
            error
                .message
                .contains("unsupported Lua value type 'function'"),
            "{new}: {error:?}"
        );
    }
}

#[test]
fn test_completion_delegate_宣言のないsessionのchild参照をyamlと同じ診断にする() {
    // Given
    let lua = DELEGATE_LUA
        .replace("work.delegate{ child = check, inputs = { result = work }, when = work.child.complete, max_iterations = 3 }", "")
        .replace("node = work, inputs = { task = r.request } }", "node = work, inputs = { task = r.request } }, r.child{node = check, inputs = {result = work.child.complete}}");
    let yaml = DELEGATE_YAML
        .replace("      delegate: {child: check, inputs: {result: work}, when: child.complete, max_iterations: 3}\n", "")
        .replace("        - work: {inputs: {task: request}}", "        - work: {inputs: {task: request}}\n        - check: {inputs: {result: work.child.complete}}");
    // When / Then
    assert_delegate_equivalent(&lua, &yaml, false);
}

#[test]
fn test_completion_delegate_親inputの多段参照とrequireなしを受理する() {
    // Given
    let lua = DELEGATE_LUA
        .replace(
            "local task = r.input('task')",
            "local task = r.input('task', result)",
        )
        .replace(
            "inputs = { result = work }",
            "inputs = { result = task.task }",
        )
        .replace(", completion = { require = r.completion.approval }", "");
    let yaml = DELEGATE_YAML
        .replace("input: [task]", "input: [{task: result}]")
        .replace("inputs: {result: work}", "inputs: {result: task.task}")
        .replace("      require: approval\n", "");
    // When / Then
    assert_delegate_equivalent(&lua, &yaml, true);
}

#[test]
fn test_completion_delegate_配線先とのcontract互換検査を追加しない() {
    // Given
    let lua = DELEGATE_LUA
        .replace("r.input('result')", "r.input('result', result)")
        .replace(
            "inputs = { result = work }",
            "inputs = { result = work.child.complete }",
        );
    let yaml = DELEGATE_YAML
        .replace("input: [result]", "input: [{result: result}]")
        .replace(
            "inputs: {result: work}",
            "inputs: {result: work.child.complete}",
        );
    // When / Then
    assert_delegate_equivalent(&lua, &yaml, true);
}

#[test]
fn test_completion_delegate_不正な引数と供給元を拒否する() {
    // Given
    for (old, new, code) in [
        ("work.delegate{ child = check, inputs = { result = work }, when = work.child.complete, max_iterations = 3 }", "work.delegate('check')", "WFS002"),
        ("child = check,", "child = 'check',", "WFS002"),
        ("inputs = { result = work }", "inputs = false", "WFS002"),
        ("inputs = { result = work }", "inputs = {work}", "WFS002"),
        ("inputs = { result = work }", "inputs = {result = true}", "WFS002"),
        ("inputs = { result = work }", "inputs = {result = check}", "WFR007"),
        ("inputs = { result = work }", "inputs = {result = r.items}", "WFR007"),
        ("inputs = { result = work }", "inputs = {result = r.input('external')}", "WFR007"),
        ("when = work.child.complete", "when = true", "WFS002"),
        ("when = work.child.complete", "when = check.complete", "WFR003"),
        ("when = work.child.complete", "when = task", "WFR003"),
        ("when = work.child.complete", "when = r.request", "WFR003"),
        ("when = work.child.complete", "when = work", "WFR003"),
        ("max_iterations = 3", "[1] = true, max_iterations = 3", "WFS002"),
    ] {
        // When
        let diagnosis = diagnose_delegate_lua(&DELEGATE_LUA.replace(old, new));
        // Then
        assert!(diagnosis.has_errors(), "{new}");
        assert_eq!(diagnosis.diagnostics[0].code, code, "{new}: {:?}", diagnosis.diagnostics);
    }
}

#[test]
fn test_completion_delegate_保持したメソッドも元のsessionへ宣言する() {
    // Given
    let lua = DELEGATE_LUA.replace("work.delegate{", "local declare = work.delegate\ndeclare{");
    // When / Then
    assert_delegate_equivalent(&lua, DELEGATE_YAML, true);
}

#[test]
fn test_completion_delegate_無名sessionの自己artifactを正準名で配線し保存後も解決する() {
    // Given
    for composite in ["sequence", "fanout"] {
        for (source, expected) in [
            (
                "work",
                serde_json::json!({"done": true, "task": "task", "child": {"detail": {"clean": true}}}),
            ),
            ("work.task", serde_json::json!("task")),
            ("work.child.detail.clean", serde_json::json!(true)),
        ] {
            let lua = DELEGATE_LUA
                .replace("name = 'work', ", "")
                .replace("r.sequence{ children", &format!("r.{composite}{{ children"))
                .replace(
                    "inputs = { result = work }",
                    &format!("inputs = {{ result = {source} }}"),
                );
            // When
            let loaded = diagnose_delegate_lua(&lua);
            // Then
            assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
            let workflow = loaded.workflow.unwrap();
            let owner = workflow
                .nodes
                .iter()
                .find(|node| node.completion.delegate.is_some())
                .unwrap();
            assert_eq!(owner.name, "main#0");
            let yaml = DELEGATE_YAML
                .replace("sequence:", &format!("{composite}:"))
                .replace(
                    "inputs: {result: work}",
                    &format!("inputs: {{result: {source}}}"),
                )
                .replace("work", &owner.name);
            let mut expected_workflow: WorkflowDefinition = serde_saphyr::from_str(&yaml).unwrap();
            expected_workflow
                .nodes
                .iter_mut()
                .find(|node| node.name == "main#0")
                .unwrap()
                .completion
                .delegate
                .as_mut()
                .unwrap()
                .inputs[0]
                .1 = InputSourceRef::node_artifact(source.replace("work", "main#0"));
            assert_eq!(workflow, expected_workflow);
            let restored: WorkflowDefinition =
                serde_json::from_str(&serde_json::to_string(&workflow).unwrap()).unwrap();
            crate::domain::workflow::services::validation::validate(&restored).unwrap();
            let bindings = reference::resolve_entry_bindings(
                Some(
                    &restored
                        .node_by_name(&owner.name)
                        .unwrap()
                        .completion
                        .delegate
                        .as_ref()
                        .unwrap()
                        .child_entry(),
                ),
                &HashMap::from([(
                    owner.name.clone(),
                    serde_json::json!({"done": true, "task": "task", "child": {"detail": {"clean": true}}}),
                )]),
            );
            assert_eq!(bindings, vec![("result".into(), expected)]);
        }
    }
}

#[test]
fn test_completion_delegate_無名sessionの名前はnodeとinputに衝突しない() {
    // Given
    let lua = DELEGATE_LUA
        .replace("name = 'work', ", "")
        .replace("name = 'check'", "name = 'node-1-' ")
        .replace("r.input('task')", "r.input('node-1')")
        .replace(
            "inputs = { task = r.request }",
            "inputs = { ['node-1'] = r.request }",
        )
        .replace(
            "work.delegate{",
            "local main = r.sequence{ children = { r.child{ node = main } } }\nwork.delegate{",
        );
    // When
    let result = diagnose_delegate_lua(&lua);
    // Then
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let workflow = result.workflow.unwrap();
    let owner = workflow
        .nodes
        .iter()
        .find(|node| node.completion.delegate.is_some())
        .unwrap();
    assert_ne!(owner.name, "node-1");
    assert_ne!(owner.name, "node-1-");
    assert_eq!(owner.name, "main#0#0");
}

#[test]
fn test_completion_delegate_無名sessionの命名は自己供給と未到達draftに依存しない() {
    // Given
    let lua = DELEGATE_LUA.replace("name = 'work', ", "");
    let baseline = diagnose_delegate_lua(&lua).workflow.unwrap();
    for drafts in [
        "r.command{command = 'true'}\nr.input('unused')",
        "r.input('unused')\nr.command{name = 'node-1', command = 'true'}",
        "for i = 1, 2000 do r.command{name = 'unused-' .. i, command = 'true'}; r.input('unused-' .. i) end",
    ] {
        for insertion in ["local check =", "local work =", "return r.workflow"] {
            // When
            let result = diagnose_delegate_lua(&lua.replace(insertion, &format!("{drafts}\n{insertion}")));
            // Then
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            assert_eq!(result.workflow.unwrap(), baseline);
        }
    }
    // When
    let without_self_source =
        diagnose_delegate_lua(&lua.replace("result = work", "result = r.request"));
    // Then
    assert!(without_self_source.diagnostics.is_empty());
    assert_eq!(
        without_self_source
            .workflow
            .unwrap()
            .nodes
            .iter()
            .map(|node| &node.name)
            .collect::<Vec<_>>(),
        baseline
            .nodes
            .iter()
            .map(|node| &node.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_completion_delegate_生成名参照と内部snapshot表記をyamlで受理しない() {
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
        let result = super::super::diagnostics::diagnose_workflow_source(&yaml, None);
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
fn test_completion_delegate_複数inputsの記述順が異なってもyamlと同一定義になる() {
    // Given
    let lua = DELEGATE_LUA
        .replace(
            "r.input('result')",
            "r.input('zebra'), r.input('apple'), r.input('middle')",
        )
        .replace(
            "inputs = { result = work }",
            "inputs = { zebra = work, apple = work.task, middle = r.request }",
        );
    let yaml = DELEGATE_YAML
        .replace("input: [result]", "input: [zebra, apple, middle]")
        .replace(
            "inputs: {result: work}",
            "inputs: {zebra: work, apple: work.task, middle: request}",
        );
    // When / Then
    let workflow = assert_delegate_equivalent(&lua, &yaml, true).unwrap();
    for (old, new) in [
        ("zebra: work, ", ""),
        ("apple: work.task", "other: work.task"),
        ("apple: work.task", "apple: request"),
        ("child: check", "child: main"),
        ("when: child.complete", "when: done"),
        ("max_iterations: 3", "max_iterations: 2"),
    ] {
        let different: WorkflowDefinition =
            serde_saphyr::from_str(&yaml.replace(old, new)).unwrap();
        assert_ne!(workflow, different, "{new}");
    }
    let yaml: WorkflowDefinition = serde_saphyr::from_str(&yaml).unwrap();
    assert_eq!(
        yaml.node_by_name("work")
            .unwrap()
            .completion
            .delegate
            .as_ref()
            .unwrap()
            .inputs
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>(),
        ["zebra", "apple", "middle"]
    );
}

#[test]
fn test_completion_delegate_yamlのinputs記述順を保存復元前後のread_modelが保持する() {
    // Given
    let yaml = DELEGATE_YAML
        .replace("input: [result]", "input: [zebra, apple, middle]")
        .replace(
            "inputs: {result: work}",
            "inputs: {zebra: work, apple: work.task, middle: request}",
        );
    // When
    let loaded = super::super::diagnostics::diagnose_workflow_source(&yaml, None);
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

#[test]
fn test_completion_delegate_非sessionのhandleからの宣言を受理しない() {
    // Given
    for owner in [
        "r.command{ command = 'true' }",
        "r.sequence{ children = { r.child{ node = r.command{ command = 'true' } } } }",
        "r.fanout{ children = { r.child{ node = r.command{ command = 'true' } } } }",
    ] {
        let lua = format!("local r = require('releash')\nlocal work = {owner}\nlocal check = r.command{{ command = 'true' }}\nwork.delegate{{ child = check, when = work.child.ok, max_iterations = 3 }}\nreturn r.workflow{{ name = 'delegate', description = 'test', main = work }}");
        // When
        let result = diagnose_delegate_lua(&lua);
        // Then
        assert!(result.workflow.is_none(), "{owner}");
        assert_eq!(result.diagnostics.len(), 1, "{owner}");
        assert_eq!(result.diagnostics[0].code, "WFS010", "{owner}");
        assert_eq!(
            result.diagnostics[0].stage,
            crate::adaptor::protocol::workflow::DiagnosticStage::ParseShape
        );
        assert!(
            result.diagnostics[0].message.contains("attempt to call"),
            "{:?}",
            result.diagnostics
        );
    }
}

#[test]
fn removed_failure_policy_fields_and_helpers_are_rejected() {
    for option in [
        "on_failure = 'ignore'",
        "on_failure = { retry = 1 }",
        "on_failure = r.retry(1)",
        "on_failure = r.ignore()",
    ] {
        let source = format!(
            r#"local r = require('releash')
local work = r.command{{ name = 'work', command = 'true' }}
return r.workflow{{name = 'removed-policy', description = 'test', main = r.sequence{{children = {{r.child{{node = work, {option}}}}}}}}}
"#
        );
        assert!(load_unconsumed_source(&source).is_err(), "{option}");
    }
}
