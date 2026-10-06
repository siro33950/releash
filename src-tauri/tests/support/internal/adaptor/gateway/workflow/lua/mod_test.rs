use super::*;
use releash_lib::test_support::integration::infrastructure::lua::evaluator::LuaHost;

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
pub fn test_lua未消費参照_sequenceのchildの予約fieldを受理する() {
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
pub fn test_lua未消費参照_入れ子のsequenceとartifactの多段fieldを受理する() {
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
pub fn test_lua未消費参照_sequenceの未知のchildとfieldを添字アクセス行で拒否する() {
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
pub fn test_lua未消費参照_mainから到達しないdraftは検証しない() {
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
pub fn test_lua未消費参照_fanoutのchildの未配線参照を従来どおり受理する() {
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
pub fn test_lua未消費参照_inputのcontractを従来どおり検証する() {
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
pub fn test_lua未消費参照_fanoutの名前キーと添字キーと入れ子を受理する() {
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
pub fn test_lua未消費参照_fanoutの未知キーと非正準添字と非objectを拒否する() {
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

const PREDICATE_SOURCE: &str = include_str!(
    "../../../../../../../src/adaptor/gateway/workflow/fixtures/valid/predicate-routing.lua"
);
const PREDICATE_EXPRESSION: &str = "r.all{ judge.passed, r.any{ judge.clean, judge.skipped } }";

#[test]
pub fn test_lua参照解決_三つの利用箇所で参照起点と位置と容量計上を保持する() {
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
                    assert_eq!(host.test_child_inputs(0).len(), 1);
                    assert_eq!(host.test_child_inputs(0)[0].0, "data");
                    host.test_child_inputs(0)[0].1
                }
                _ => {
                    assert_eq!(evaluation.value, handle(HANDLE_RULE, 0));
                    match &host.test_rules()[0] {
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
            assert_eq!(host.test_sources().len(), 3 + added_sources);
            assert_eq!(host.test_predicate_entries(), 0);
            assert_eq!(host.arena_entries(), 6 + added_sources);
            let (root, path, location) = match &host.test_sources()[index] {
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
            assert_eq!(host.test_source_fields(*path), expected_fields);
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
pub fn test_lua述語解決_存在しないhandleは利用箇所のfieldと位置で拒否する() {
    // Given
    let location = LuaSourceLocation {
        source: "missing-predicate.lua".to_string(),
        line: 7,
    };
    for function in [FN_WHEN, FN_ALL, FN_ANY] {
        let (mut host, _) = predicate_budget_host("source");
        let before = host.arena_entries();
        let value = handle(HANDLE_PREDICATE, host.test_predicates().len());
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
        assert_eq!(host.test_predicate_entries(), 0);
        assert!(host.test_predicates().is_empty());
        assert!(host.test_rules().is_empty());
    }
}

#[test]
pub fn test_lua単一参照のwhen_rule一件だけ計上してbaseの容量境界を保つ() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 7,
    };
    for current in [None, Some(99_999), Some(100_000)] {
        let (mut host, source) = predicate_budget_host("source");
        let source_index = test_handle_index(&source, HANDLE_SOURCE).unwrap();
        if let Some(current) = current {
            *host.test_predicate_entries_mut() = current - host.arena_entries();
        }
        let before = host.arena_entries();
        let predicate_entries = host.test_predicate_entries();
        let sources = host.test_sources().len();

        // When
        let result = host.call(FN_WHEN, when_arguments(source), location.clone());

        // Then
        assert_eq!(host.test_sources().len(), sources);
        assert!(host.test_predicates().is_empty());
        assert_eq!(host.test_predicate_entries(), predicate_entries);
        if before < MAX_HOST_ARENA_ENTRIES {
            assert_eq!(result.unwrap(), handle(HANDLE_RULE, 0));
            assert_eq!(host.arena_entries(), before + 1);
            assert_eq!(host.test_rules().len(), 1);
            let RuleDraft::When { on, on_true, next } = &host.test_rules()[0] else {
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
            assert!(host.test_rules().is_empty());
        }
    }
}

#[test]
pub fn test_lua合成述語_同じsourceの各refと根を計上して上限で拒否する() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 7,
    };
    for function in [FN_ALL, FN_ANY] {
        for count in [1, 3] {
            for remaining in [None, Some(count + 1), Some(count)] {
                let (mut host, source) = predicate_budget_host("source");
                let source_index = test_handle_index(&source, HANDLE_SOURCE).unwrap();
                if let Some(remaining) = remaining {
                    *host.test_predicate_entries_mut() =
                        MAX_HOST_ARENA_ENTRIES - remaining - host.arena_entries();
                }
                let before = host.arena_entries();
                let predicate_entries = host.test_predicate_entries();
                let sources = host.test_sources().len();
                let arguments = vec![LuaData::Table(LuaTableData {
                    entries: (1..=count)
                        .map(|index| (LuaTableKey::Integer(index as i64), source.clone()))
                        .collect(),
                })];

                // When
                let result = host.call(function, arguments, location.clone());

                // Then
                assert_eq!(host.test_sources().len(), sources);
                assert!(host.test_rules().is_empty());
                if remaining != Some(count) {
                    assert_eq!(result.unwrap(), handle(HANDLE_PREDICATE, 0));
                    assert_eq!(host.arena_entries(), before + count + 1);
                    assert_eq!(host.test_predicate_entries(), predicate_entries + count + 1);
                    let refs = vec![Predicate::Ref(source_index); count];
                    let expected = if function == FN_ALL {
                        Predicate::And(refs)
                    } else {
                        Predicate::Or(refs)
                    };
                    assert_eq!(*host.test_predicates(), vec![(expected, count + 1)]);
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
                    assert!(host.test_predicates().is_empty());
                }
            }
        }
    }
}

#[test]
pub fn test_lua述語再利用_合成とwhenは複製全体と一件を計上して上限で拒否する() {
    // Given
    let location = LuaSourceLocation {
        source: "budget.lua".to_string(),
        line: 7,
    };
    for function in [FN_ALL, FN_ANY, FN_WHEN] {
        for remaining in [None, Some(6), Some(5), Some(4)] {
            let (mut host, value) =
                predicate_budget_host("r.all{ source, r.any{ source, source } }");
            let (predicate, size) = host.test_predicates()[1].clone();
            assert_eq!(size, 5);
            assert_eq!(host.test_predicate_entries(), 8);
            if let Some(remaining) = remaining {
                *host.test_predicate_entries_mut() +=
                    MAX_HOST_ARENA_ENTRIES - remaining - host.arena_entries();
            }
            let before = host.arena_entries();
            let predicate_entries = host.test_predicate_entries();
            let sources = host.test_sources().len();
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
            assert_eq!(host.test_sources().len(), sources);
            if remaining.is_none_or(|remaining| remaining > size) {
                assert_eq!(host.arena_entries(), before + size + 1);
                if function == FN_WHEN {
                    assert_eq!(result.unwrap(), handle(HANDLE_RULE, 0));
                    assert_eq!(host.test_predicate_entries(), predicate_entries + size);
                    assert_eq!(host.test_predicates().len(), 2);
                    assert_eq!(host.test_rules().len(), 1);
                    let RuleDraft::When { on, on_true, next } = &host.test_rules()[0] else {
                        panic!("expected when rule");
                    };
                    assert_eq!(*on, predicate);
                    assert_eq!((*on_true, *next), (1, 2));
                } else {
                    assert_eq!(result.unwrap(), handle(HANDLE_PREDICATE, 2));
                    assert_eq!(host.test_predicate_entries(), predicate_entries + size + 1);
                    assert!(host.test_rules().is_empty());
                    let expected = if function == FN_ALL {
                        Predicate::And(vec![predicate])
                    } else {
                        Predicate::Or(vec![predicate])
                    };
                    assert_eq!(host.test_predicates()[2], (expected, size + 1));
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
                assert_eq!(host.test_predicate_entries(), predicate_entries + added);
                assert!(host.test_rules().is_empty());
                assert_eq!(host.test_predicates().len(), 2);
            }
        }
    }
}

#[test]
pub fn test_lua述語_不正な要素と自child以外の参照起点を拒否する() {
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
pub fn test_lua述語_switchのsourceには述語を受理しない() {
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
pub fn test_lua述語_再利用による展開もarena上限に数える() {
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
pub fn test_completion要求_luaのrequire値は承認handleだけを受理する() {
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
pub fn test_completion_delegate_例を受理し承認とchildを含むyamlと同一定義になる() {
    // Given / When
    let workflow = assert_delegate_equivalent(DELEGATE_LUA, DELEGATE_YAML, true).unwrap();
    // Then
    assert_eq!(workflow.nodes.len(), 3);
    let completion = &workflow.node_by_name("work").unwrap().completion;
    assert!(completion.requires_approval());
    assert_eq!(completion.delegate.as_ref().unwrap().child, "check");
}

#[test]
pub fn test_completion_delegate_inputs省略と全供給元と述語をyamlと同じ配線にする() {
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
pub fn test_completion_delegate_必須field未知キーと回数のdiagnosticがyamlと一致する() {
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
pub fn test_completion_delegate_artifact欠落とchildの自己参照root共有をyamlと同じ診断にする() {
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
pub fn test_completion_delegate_childの全kindと統合mapをyamlと同じcontractで検査する() {
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
pub fn test_completion_delegate_inputsとwhenの存在しないfieldと不適合型を拒否する() {
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
pub fn test_completion_delegate_二重宣言をshapeエラーにしてfield名よりメソッドを優先する() {
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
        crate::usecase::workflow::diagnostic_dto::DiagnosticStage::ParseShape
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
pub fn test_completion_delegate_同名fieldがあってもメソッドを供給元として受理しない() {
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
            crate::usecase::workflow::diagnostic_dto::DiagnosticStage::ParseShape,
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
pub fn test_completion_delegate_宣言のないsessionのchild参照をyamlと同じ診断にする() {
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
pub fn test_completion_delegate_親inputの多段参照とrequireなしを受理する() {
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
pub fn test_completion_delegate_配線先とのcontract互換検査を追加しない() {
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
pub fn test_completion_delegate_不正な引数と供給元を拒否する() {
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
pub fn test_completion_delegate_保持したメソッドも元のsessionへ宣言する() {
    // Given
    let lua = DELEGATE_LUA.replace("work.delegate{", "local declare = work.delegate\ndeclare{");
    // When / Then
    assert_delegate_equivalent(&lua, DELEGATE_YAML, true);
}

#[test]
pub fn test_completion_delegate_無名sessionの自己artifactを正準名で配線し保存後も解決する() {
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
pub fn test_completion_delegate_無名sessionの名前はnodeとinputに衝突しない() {
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
pub fn test_completion_delegate_無名sessionの命名は自己供給と未到達draftに依存しない() {
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
pub fn test_completion_delegate_複数inputsの記述順が異なってもyamlと同一定義になる() {
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
pub fn test_completion_delegate_yamlのinputs記述順を保存復元前後のread_modelが保持する() {
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
pub fn test_completion_delegate_非sessionのhandleからの宣言を受理しない() {
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
            crate::usecase::workflow::diagnostic_dto::DiagnosticStage::ParseShape
        );
        assert!(
            result.diagnostics[0].message.contains("attempt to call"),
            "{:?}",
            result.diagnostics
        );
    }
}

pub(crate) mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::super::*;

    fn load(source: &str) -> Result<LuaWorkflowDefinition, LuaWorkflowError> {
        let directory = TempDir::new().unwrap();
        load_lua_workflow(
            "review.lua",
            source,
            directory.path(),
            LuaFacetCatalog::default(),
        )
    }

    #[test]
    pub fn builds_a_workflow_and_synthesizes_child_names() {
        let loaded = load(
            r#"
local r = require("releash")
local first = r.command{ command = "echo first" }
local second = r.command{ command = "echo second" }
return r.workflow{
  name = "review",
  description = "Review",
  main = r.sequence{
    children = {
      r.child{ node = first },
      r.child{ node = second },
    },
  },
}
"#,
        )
        .unwrap();

        assert_eq!(loaded.workflow.entry, "main");
        assert_eq!(loaded.workflow.nodes[0].name, "main");
        assert!(loaded.workflow.node_by_name("main#0").is_some());
        assert!(loaded.workflow.node_by_name("main#1").is_some());
    }

    #[test]
    pub fn rejects_same_node_value_in_multiple_children() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{ command = "echo" }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = child }, r.child{ node = child },
  } },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFC007");
    }

    #[test]
    pub fn rejects_unknown_builder_field_at_call_line() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{ command = "echo", unknown = true }
return r.workflow{ name = "review", description = "Review", main = child }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFS002");
        assert_eq!(error.location.unwrap().line, 3);
    }

    #[test]
    pub fn test_lua_session_permission_4値からworkflow_definitionを構築する() {
        for (value, expected) in [
            ("manual", SessionPermission::Manual),
            ("auto", SessionPermission::Auto),
            ("bypass", SessionPermission::Bypass),
            ("read-only", SessionPermission::ReadOnly),
        ] {
            let source = format!(
                r#"
local r = require("releash")
return r.workflow{{ name = "review", description = "Review", main = r.session{{ provider = r.provider.claude, permission = "{value}" }} }}
"#
            );

            let loaded = load(&source).unwrap();
            assert_eq!(
                loaded
                    .workflow
                    .node_by_name("main")
                    .unwrap()
                    .session()
                    .unwrap()
                    .permission,
                Some(expected)
            );
        }
    }

    #[test]
    pub fn test_lua_session_permission_未知値とprovider固有値をwfs002で拒否する() {
        for invalid in [
            "unknown",
            "acceptEdits",
            "danger-full-access",
            "workspace-write",
            "bypassPermissions",
            "plan",
        ] {
            let source = format!(
                r#"
local r = require("releash")
return r.workflow{{ name = "review", description = "Review", main = r.session{{ provider = r.provider.claude, permission = "{invalid}" }} }}
"#
            );

            let error = load(&source).unwrap_err();
            assert_eq!(error.code, "WFS002");
            assert_eq!(error.field.as_deref(), Some("permission"));
            assert!(error.message.contains(invalid));
            assert_eq!(error.location.unwrap().line, 3);
        }
    }

    #[test]
    pub fn maps_require_failures_and_non_workflow_returns_to_spec_codes() {
        let directory = TempDir::new().unwrap();
        let require_error = load_lua_workflow(
            "review.lua",
            "local value = require('../outside')\nreturn value",
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();
        let return_error = load_lua_workflow(
            "review.lua",
            "return {}",
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();

        assert_eq!(require_error.code, "WFS011");
        assert_eq!(require_error.location.unwrap().line, 1);
        assert_eq!(return_error.code, "WFS010");
        assert_eq!(return_error.location.unwrap().line, 1);
    }

    #[test]
    pub fn rejects_unknown_artifact_field_at_index_line() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{
  command = "echo",
  artifact = r.schema.object{ properties = { ok = r.schema.boolean() } },
}
local invalid = child.missing
return r.workflow{ name = "review", description = "Review", main = child }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 7);
    }

    #[test]
    pub fn rejects_unknown_facet_at_reference_line() {
        let directory = TempDir::new().unwrap();
        let error = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local f = require("facets")
local child = r.session{
  provider = r.provider.claude,
  facets = { instruction = f.instruction.missing },
}
return r.workflow{ name = "review", description = "Review", main = child }
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR900");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 6);
    }

    #[test]
    pub fn reports_reference_error_at_required_component_file_and_line() {
        let directory = TempDir::new().unwrap();
        let component = directory.path().join("component.lua");
        fs::write(
            &component,
            r#"
local r = require("releash")
return function()
  local child = r.command{ command = "echo" }
  local invalid = child.missing
  return child
end
"#,
        )
        .unwrap();

        let error = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local component = require("component")
return r.workflow{ name = "review", description = "Review", main = component() }
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();
        let location = error.location.unwrap();

        assert_eq!(error.code, "WFR003");
        assert_eq!(
            location.source,
            fs::canonicalize(component).unwrap().to_string_lossy()
        );
        assert_eq!(location.line, 5);
    }

    #[test]
    pub fn require_component_function_creates_independent_nodes() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("component.lua"),
            r#"
local r = require("releash")
return function(command)
  local leaf = r.command{ command = command }
  return r.sequence{
    completion = { require = r.completion.approval },
    children = { r.child{ node = leaf } },
  }
end
"#,
        )
        .unwrap();
        let loaded = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local component = require("component")
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = component("one") },
    r.child{ node = component("two") },
  } },
}
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap();

        assert_eq!(loaded.workflow.nodes.len(), 5);
        assert_eq!(
            loaded
                .workflow
                .nodes
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["main", "main#0", "main#0#0", "main#1", "main#1#0"]
        );
        assert_eq!(
            loaded.workflow.node_by_name("main#0").unwrap().completion,
            NodeCompletion::require_approval()
        );
        assert!(loaded
            .workflow
            .node_by_name("main#0")
            .unwrap()
            .is_sequence());
    }

    #[test]
    pub fn builds_schema_fanout_items_and_scoped_item_wiring() {
        let directory = TempDir::new().unwrap();
        let loaded = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local f = require("facets")
local topic = r.schema.string{}
local detail = r.schema.object{
  name = "topic-detail",
  properties = { label = r.schema.string{} },
}
local payload = r.schema.object{
  properties = {
    topics = r.schema.array{ items = topic },
    detail = detail,
  },
  required = { "topics" },
}
local scan = r.command{
  command = "scan",
  artifact = r.schema.object{
    properties = { payload = payload },
  },
}
local worker = r.session{
  provider = r.provider.codex,
  facets = { instruction = f.instruction.review },
  input = { r.input("topic", topic) },
}
local spread = r.fanout{
  items = scan.payload.topics,
  children = {
    r.child{ node = worker, inputs = { topic = r.items } },
  },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = scan },
    r.child{ node = spread },
  } },
}
"#,
            directory.path(),
            LuaFacetCatalog {
                instruction: vec!["review".to_string()],
                ..LuaFacetCatalog::default()
            },
        )
        .unwrap();

        assert_eq!(
            loaded
                .workflow
                .nodes
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["main", "main#0", "main#1", "main#1#0"]
        );
        let validation_errors = crate::domain::workflow::validation::validate_all(&loaded.workflow);
        assert!(validation_errors.is_empty(), "{validation_errors:#?}");
        assert!(loaded.workflow.schemas.contains_key("topic-detail"));
    }

    #[test]
    pub fn test_lua多段参照_whenとswitchを共有domain検証へ渡せる() {
        let loaded = load(
            r#"
local r = require("releash")
local route = r.schema.object{ properties = {
  flag = r.schema.boolean(),
  status = r.schema.string{ enum = { "A" } },
}, required = { "flag", "status" } }
local result = r.schema.object{ properties = { route = route } }
local yes = r.command{ name = "yes", command = "yes" }
local no = r.command{ name = "no", command = "no" }
local when_source = r.command{ name = "when-source", command = "source", artifact = result }
local switch_source = r.command{ name = "switch-source", command = "source", artifact = result }
return r.workflow{ name = "routing", description = "routing", main = r.sequence{ children = {
  r.child{ node = when_source, rules = {
    r.when{ on = when_source.route.flag, on_true = switch_source, next = switch_source },
  } },
  r.child{ node = switch_source, rules = {
    r.switch{ on = switch_source.route.status, cases = { A = yes }, next = no },
  } },
  r.child{ node = yes },
  r.child{ node = no },
} } }
"#,
        )
        .unwrap();

        let errors = crate::domain::workflow::validation::validate_all(&loaded.workflow);

        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    pub fn rejects_input_source_from_outer_composite_scope() {
        let error = load(
            r#"
local r = require("releash")
local outer = r.input("outer")
local leaf = r.command{ command = "echo", input = { r.input("value") } }
local inner = r.sequence{
  children = { r.child{ node = leaf, inputs = { value = outer } } },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{
    input = { outer },
    children = { r.child{ node = inner } },
  },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR007");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 6);
    }

    #[test]
    pub fn accepts_field_reference_from_composite_input_contract() {
        let loaded = load(
            r#"
local r = require("releash")
local payload = r.input("payload", r.schema.object{
  properties = { message = r.schema.string{} },
  required = { "message" },
})
local leaf = r.command{ command = "echo", input = { r.input("value") } }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{
    input = { payload },
    children = { r.child{ node = leaf, inputs = { value = payload.message } } },
  },
}
"#,
        )
        .unwrap();

        let sequence = loaded.workflow.entry_node().unwrap().sequence().unwrap();
        assert_eq!(sequence.children[0].inputs[0].1.raw(), "payload.message");
    }

    #[test]
    pub fn test_lua多段参照_child配線を保持して実行時に末端値を解決する() {
        // Given
        let loaded = load(
            r#"
local r = require("releash")
local result = r.schema.object{ properties = {
  payload = r.schema.object{ properties = {
    nested = r.schema.object{ properties = { title = r.schema.string{} } },
  } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target", input = { r.input("title") } }
return r.workflow{ name = "wiring", description = "wiring", main = r.sequence{ children = {
  r.child{ node = source },
  r.child{ node = target, inputs = { title = source.payload.nested.title } },
} } }
"#,
        )
        .unwrap();
        let validation_errors = crate::domain::workflow::validation::validate_all(&loaded.workflow);
        let sequence = loaded.workflow.entry_node().unwrap().sequence().unwrap();
        let target_entry = &sequence.children[1];
        let artifacts = HashMap::from([(
            "source".to_string(),
            serde_json::json!({"payload": {"nested": {"title": "resolved"}}}),
        )]);

        // When
        let bindings = crate::domain::workflow::services::reference::resolve_entry_bindings(
            Some(target_entry),
            &artifacts,
        );

        // Then
        assert!(validation_errors.is_empty(), "{validation_errors:#?}");
        assert_eq!(
            target_entry.inputs[0].1.raw(),
            "source.payload.nested.title"
        );
        assert_eq!(
            bindings,
            vec![("title".to_string(), serde_json::json!("resolved"))]
        );
    }

    #[test]
    pub fn rejects_untyped_input_field_in_child_wiring_at_the_index_line() {
        let error = load(
            r#"
local r = require("releash")
local payload = r.input("payload")
local leaf = r.command{ command = "echo", input = { r.input("value") } }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{
    input = { payload },
    children = { r.child{ node = leaf, inputs = { value = payload.message } } },
  },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        assert_eq!(error.message, "input does not declare a contract");
        assert_eq!(error.location.unwrap().line, 9);
    }

    #[test]
    pub fn test_lua_child配線_owner外の型なしinput_fieldをindex行のwfr003で拒否する() {
        let error = load(
            r#"
local r = require("releash")
local payload = r.input("payload")
local leaf = r.command{ command = "echo", input = { r.input("value") } }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{
    children = { r.child{ node = leaf, inputs = { value = payload.message } } },
  },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        assert_eq!(error.message, "input does not declare a contract");
        assert_eq!(error.location.unwrap().line, 8);
    }

    #[test]
    pub fn rejects_named_main_node() {
        let error = load(
            r#"
local r = require("releash")
return r.workflow{
  name = "review", description = "Review",
  main = r.command{ name = "root", command = "true" },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFS006");
        assert_eq!(error.location.unwrap().line, 5);
    }

    #[test]
    pub fn rejects_missing_main_with_existing_resolve_diagnostic() {
        let error = load(
            r#"
local r = require("releash")
return r.workflow{ name = "review", description = "Review" }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR006");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 3);
    }

    #[test]
    pub fn test_lua未消費参照_全段が存在する多段fieldを受理する() {
        let loaded = load(
            r#"
local r = require("releash")
local child = r.command{
  command = "echo",
  artifact = r.schema.object{ properties = {
    nested = r.schema.object{ properties = { value = r.schema.string{} } },
  } },
}
local nested = child.nested.value
return r.workflow{ name = "review", description = "Review", main = child }
"#,
        )
        .unwrap();

        assert_eq!(loaded.workflow.entry_node().unwrap().name, "main");
    }

    #[test]
    pub fn test_lua未消費参照_存在しない段をwfr003で拒否する() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{
  command = "echo",
  artifact = r.schema.object{ properties = {
    nested = r.schema.object{ properties = { value = r.schema.string{} } },
  } },
}
local invalid = child.nested.missing
return r.workflow{ name = "review", description = "Review", main = child }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        assert_eq!(error.message, "artifact field 'missing' does not exist");
        assert_eq!(error.location.unwrap().line, 9);
    }

    #[test]
    pub fn test_lua消費済み参照_多段artifactとinputを共有domain検証へ渡す() {
        let loaded = load(
            r#"
local r = require("releash")
local text = r.schema.string{}
local payload = r.schema.object{
  name = "payload",
  properties = { nested = r.schema.object{ properties = { value = text } } },
}
local source = r.command{
  name = "source", command = "source",
  artifact = r.schema.object{ properties = { payload = payload } },
}
local input = r.input("input", payload)
local target = r.command{
  name = "target", command = "echo {{ input.nested.value }}",
  input = { input }, env = { VALUE = input.nested.value },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = source },
    r.child{ node = target, inputs = { input = source.payload } },
  } },
}
"#,
        )
        .unwrap();

        let errors = crate::domain::workflow::validation::validate_all(&loaded.workflow);
        assert!(errors.is_empty(), "{errors:#?}");
        let target = loaded.workflow.node_by_name("target").unwrap();
        assert_eq!(
            target
                .command_spec()
                .unwrap()
                .env
                .values()
                .next()
                .unwrap()
                .as_string(),
            "input.nested.value"
        );
    }

    #[test]
    pub fn rejects_non_field_fanout_items_at_the_builder_line() {
        let error = load(
            r#"
local r = require("releash")
local source = r.command{ command = "source" }
local child = r.command{ command = "child" }
local spread = r.fanout{
  items = source,
  children = { r.child{ node = child } },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = source }, r.child{ node = spread },
  } },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        assert_eq!(error.location.unwrap().line, 5);
    }

    #[test]
    pub fn rejects_fanout_items_with_a_non_reference_segment() {
        let error = load(
            r#"
local r = require("releash")
local source = r.command{
  name = "source",
  command = "source",
  artifact = r.schema.object{ properties = {
    ["legacy values"] = r.schema.array{ items = r.schema.string{} },
  } },
}
local child = r.command{ name = "child", command = "child" }
local spread = r.fanout{
  name = "spread",
  items = source["legacy values"],
  children = { r.child{ node = child } },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = source }, r.child{ node = spread },
  } },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        assert_eq!(error.message, "invalid fanout items field path");
    }

    #[test]
    pub fn lua_definition_equals_the_same_yaml_definition_without_origin_metadata() {
        let loaded = load(
            r#"
local r = require("releash")
local result = r.schema.object{
  name = "result",
  properties = { message = r.schema.string{} },
  required = { "message" },
}
local inspect = r.command{
  name = "inspect",
  command = "echo inspect",
  artifact = result,
  input = { r.input("request_text") },
  completion = { require = r.completion.approval },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = inspect, inputs = { request_text = r.request } },
  } },
}
"#,
        )
        .unwrap();
        let yaml: WorkflowDefinition = serde_saphyr::from_str(
            r#"
name: review
description: Review
schemas:
  result:
    type: object
    properties:
      message: string
    required:
      - message
nodes:
  main:
    sequence:
      children:
        - inspect:
            command: echo inspect
            artifact: result
            input:
              - request_text
            completion:
              require: approval
            inputs:
              request_text: request
"#,
        )
        .unwrap();

        assert_eq!(loaded.workflow, yaml);

        use crate::domain::workflow::entities::workflow_execution::{
            ExecutionTree, ExecutionTreeRestore,
        };
        let mut lua_execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
            id: "execution".to_string(),
            workflow: loaded.workflow,
            ..ExecutionTreeRestore::default()
        });
        let mut yaml_execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
            id: "execution".to_string(),
            workflow: yaml,
            ..ExecutionTreeRestore::default()
        });
        let mut lua_index = 0_u32;
        let mut yaml_index = 0_u32;
        let lua_started = lua_execution
            .start_root(
                &mut || {
                    lua_index += 1;
                    format!("node-{lua_index}")
                },
                1.0,
            )
            .unwrap();
        let yaml_started = yaml_execution
            .start_root(
                &mut || {
                    yaml_index += 1;
                    format!("node-{yaml_index}")
                },
                1.0,
            )
            .unwrap();

        assert_eq!(lua_started, yaml_started);
        assert_eq!(lua_execution, yaml_execution);
    }

    #[test]
    pub fn builds_all_rule_and_completion_variants() {
        let loaded = load(
            r#"
local r = require("releash")
local check = r.command{
  command = "check",
}
local classify = r.command{
  command = "classify",
  completion = { require = r.completion.approval },
  artifact = r.schema.object{
    properties = { status = r.schema.string{ enum = { "done", "retry" } } },
    required = { "status" },
  },
}
local retry = r.command{ command = "retry" }
local done = r.command{ command = "done" }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{
      node = check,
      rules = { r.when{ on = check.ok, on_true = classify, next = retry } },
    },
    r.child{
      node = classify,
      rules = { r.switch{ on = classify.status, cases = {
        done = done,
        retry = retry,
      }, next = done } },
    },
    r.child{
      node = retry,
      rules = {
        r.loop_guard{ max_iterations = 3, on_exhausted = done },
        r.next(check),
      },
    },
    r.child{ node = done, rules = {} },
  } },
}
"#,
        )
        .unwrap();

        let main = loaded.workflow.root_sequence().unwrap();
        assert!(matches!(
            main.children[0].rules.as_deref(),
            Some([Rule::When { .. }])
        ));
        assert!(matches!(
            main.children[1].rules.as_deref(),
            Some([Rule::Switch { .. }])
        ));
        assert!(matches!(
            main.children[2].rules.as_deref(),
            Some([Rule::LoopGuard { .. }, Rule::Next(_)])
        ));
        assert_eq!(main.children[3].rules.as_deref(), Some(&[][..]));
        assert_eq!(
            loaded.workflow.node_by_name("main#1").unwrap().completion,
            NodeCompletion::require_approval()
        );
        let errors = crate::domain::workflow::validation::validate_all(&loaded.workflow);
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    pub fn runtime_modules_ignore_missing_or_stale_editor_stubs() {
        let directory = TempDir::new().unwrap();
        let source = r#"
local r = require("releash")
local f = require("facets")
return r.workflow{
  name = "review", description = "Review",
  main = r.session{
    provider = r.provider.claude,
    facets = { instruction = f.instruction.live },
  },
}
"#;
        let catalog = || LuaFacetCatalog {
            instruction: vec!["live".to_string()],
            ..LuaFacetCatalog::default()
        };
        let without_stubs =
            load_lua_workflow("review.lua", source, directory.path(), catalog()).unwrap();
        fs::create_dir_all(directory.path().join(".releash")).unwrap();
        fs::write(
            directory.path().join(".releash/releash.lua"),
            "error('stale runtime stub must not run')",
        )
        .unwrap();
        fs::write(
            directory.path().join(".releash/facets.lua"),
            "return { instruction = {} }",
        )
        .unwrap();

        let with_stale_stubs =
            load_lua_workflow("review.lua", source, directory.path(), catalog()).unwrap();

        assert_eq!(with_stale_stubs.workflow, without_stubs.workflow);
    }

    #[test]
    pub fn repeated_loads_of_the_same_file_group_are_deterministic() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("component.lua"),
            r#"
local r = require("releash")
return function(label)
  return r.command{ command = "echo " .. label }
end
"#,
        )
        .unwrap();
        let source = r#"
local r = require("releash")
local component = require("component")
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = component("one") },
    r.child{ node = component("two") },
  } },
}
"#;

        let first = load_lua_workflow(
            "review.lua",
            source,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap();
        let second = load_lua_workflow(
            "review.lua",
            source,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap();

        assert_eq!(second.workflow, first.workflow);
    }

    #[test]
    pub fn rejects_definitions_that_exhaust_the_host_arena_budget() {
        let directory = TempDir::new().unwrap();

        let error = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
for _ = 1, 200000 do
  r.command{ command = "x" }
end
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = { r.child{ node = r.command{ command = "true" } } } },
}
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();

        assert_eq!(error.code, "WFS010");
        assert!(error.message.contains("builder values"));
    }

    #[test]
    pub fn rejects_a_single_child_whose_inputs_exhaust_the_arena_budget() {
        let directory = TempDir::new().unwrap();

        let error = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local target = r.command{ command = "x" }
-- arena を上限の手前まで埋めてから、1 回の r.child で残りを超える inputs を渡す。
for _ = 1, 99000 do
  r.command{ command = "x" }
end
local inputs = {}
for i = 1, 5000 do
  inputs["p" .. i] = target
end
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = { r.child{ node = target, inputs = inputs } } },
}
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();

        assert_eq!(error.code, "WFS010");
        assert!(error.message.contains("builder values"));
    }

    #[test]
    pub fn resource_limits_are_reported_as_wfs010_without_poisoning_following_loads() {
        let directory = TempDir::new().unwrap();
        let infinite = load_lua_workflow_with_limits(
            "infinite.lua",
            "while true do end",
            directory.path(),
            LuaFacetCatalog::default(),
            LuaLimits {
                memory_bytes: 64 * 1024 * 1024,
                instructions: 20_000,
            },
        )
        .unwrap_err();
        let oversized = load_lua_workflow_with_limits(
            "oversized.lua",
            "return string.rep('x', 16777216)",
            directory.path(),
            LuaFacetCatalog::default(),
            LuaLimits {
                memory_bytes: 4 * 1024 * 1024,
                instructions: 50_000_000,
            },
        )
        .unwrap_err();
        let following = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
return r.workflow{
  name = "review", description = "Review",
  main = r.command{ command = "true" },
}
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        );

        assert_eq!(infinite.code, "WFS010");
        assert!(infinite.message.contains("instruction limit"));
        assert_eq!(oversized.code, "WFS010");
        assert!(oversized.message.contains("memory limit"));
        assert!(following.is_ok());
    }
}
