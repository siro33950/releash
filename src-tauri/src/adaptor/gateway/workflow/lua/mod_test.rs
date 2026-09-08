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
            assert_eq!(host.arena_entries(), 7 + added_sources);
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
