use super::*;

const MERGED_REFERENCES: &str = include_str!(
    "../../../../../../src/adaptor/gateway/workflow/fixtures/valid/sequence-merged-references.yml"
);

#[test]
pub fn test_lua未消費参照の診断_非object契約のleafを唯一の診断で指す() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let source = r#"local r = require('releash')
local bad = r.command{ name = 'bad', command = 'check', artifact = r.schema.string{} }
local part = r.sequence{ name = 'part', children = { r.child{ node = bad } } }
local main = r.sequence{ children = { r.child{ node = part } } }
local ref = main.part.bad.ok
return r.workflow{ name = 'nonobject-leaf', description = 'test', main = main }
"#;

    // When
    let diagnosis =
        diagnose_lua_workflow_source("nonobject-leaf.lua", source, tmp.path(), tmp.path(), None);

    // Then
    assert!(diagnosis.workflow.is_none());
    assert_eq!(
        diagnosis.diagnostics.len(),
        1,
        "{:?}",
        diagnosis.diagnostics
    );
    let diagnostic = &diagnosis.diagnostics[0];
    assert_eq!(diagnostic.code, "WFR003");
    assert_eq!(diagnostic.stage, DiagnosticStage::Resolve);
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "artifact field 'bad' cannot be read from a non-object schema"
    );
}

#[test]
pub fn test_sequence宣言の診断_luaの入れ子とrequire先のartifact位置を指す() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let component = r#"local r = require("releash")
return function()
  local result = r.schema.object{ properties = {} }
  local nested = r.sequence({
    name = "nested",
    children = { r.child{ node = r.command{
      command = [[artifact = ignored, }]],
      artifact = result,
    } } },
    -- artifact = ignored
    artifact = result,
  })
  return r.sequence{
    children = { r.child{ node = nested } },
    artifact = result,
  }
end
"#;
    std::fs::create_dir(tmp.path().join("parts")).unwrap();
    std::fs::write(tmp.path().join("parts/sequence.lua"), component).unwrap();
    let inline = format!(
        "local component = (function()\n{component}\nend)()\nlocal r = require('releash')\nreturn r.workflow{{ name = 'review', description = 'test', main = component() }}"
    );
    let imported = "local r = require('releash')\nreturn r.workflow{ name = 'review', description = 'test', main = require('parts.sequence')() }";
    for (source, expected_source, offset) in [
        (inline.as_str(), "review.lua", 1),
        (imported, "parts/sequence.lua", 0),
    ] {
        // When
        let diagnosis = diagnose_lua_workflow_source(
            "review.lua",
            source,
            tmp.path(),
            tmp.path(),
            Some("review"),
        );

        // Then
        assert_eq!(
            diagnosis.diagnostics.len(),
            2,
            "{:?}",
            diagnosis.diagnostics
        );
        for (node, line, col) in [("nested", 11 + offset, 5), ("main", 15 + offset, 5)] {
            let diagnostic = diagnosis
                .diagnostics
                .iter()
                .find(|item| item.node_name.as_deref() == Some(node))
                .unwrap();
            assert_eq!(diagnostic.code, "WFS008");
            assert_eq!(diagnostic.stage, DiagnosticStage::ParseShape);
            assert_eq!(diagnostic.severity, Severity::Error);
            assert_eq!(diagnostic.field.as_deref(), Some("artifact"));
            assert_eq!(
                diagnostic.span,
                Some(DiagnosticSpan {
                    source: Some(expected_source.to_string()),
                    start_line: line,
                    start_col: col,
                    end_line: line,
                    end_col: col + 8,
                })
            );
        }
    }
}

#[test]
pub fn test_sequence宣言の診断_yamlとluaでoutputとartifactを同じcodeとstageで拒否する() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let cases = [
        (
            "output",
            "WFS002",
            include_str!("../../../../../../src/adaptor/gateway/workflow/fixtures/invalid/WFS002_sequence-output.yml"),
        ),
        (
            "artifact",
            "WFS008",
            include_str!("../../../../../../src/adaptor/gateway/workflow/fixtures/invalid/WFS008_sequence-artifact.yml"),
        ),
    ];
    for (field, code, yaml) in cases {
        let option = if field == "output" {
            "output = check"
        } else {
            "artifact = result"
        };
        let name = format!("sequence-{field}");
        let lua = format!(
            r#"local r = require("releash")
local result = r.schema.object{{ name = "result", properties = {{ passed = r.schema.boolean() }}, required = {{ "passed" }} }}
local check = r.command{{ name = "check", command = "check", artifact = result }}
return r.workflow{{ name = "{name}", description = "test", main = r.sequence{{
    {option},
    children = {{ r.child{{ node = check }} }},
}} }}
"#
        );

        // When
        let yaml_diagnosis = diagnose_workflow_source(yaml, None);
        let lua_diagnosis = diagnose_lua_workflow_source(
            &format!("{name}.lua"),
            &lua,
            tmp.path(),
            tmp.path(),
            None,
        );

        // Then
        for diagnosis in [&yaml_diagnosis, &lua_diagnosis] {
            let errors: Vec<_> = diagnosis
                .diagnostics
                .iter()
                .filter(|item| item.severity == Severity::Error)
                .collect();
            assert_eq!(errors.len(), 1, "{:?}", diagnosis.diagnostics);
            assert_eq!(errors[0].code, code);
            assert_eq!(errors[0].stage, DiagnosticStage::ParseShape);
            assert!(errors[0].span.is_some());
        }
        if field == "artifact" {
            let diagnostic = &yaml_diagnosis.diagnostics[0];
            assert_eq!(diagnostic.field.as_deref(), Some("artifact"));
            assert_eq!(diagnostic.node_name.as_deref(), Some("main"));
            assert_eq!(
                diagnostic.span.as_ref().unwrap().start_line,
                yaml.lines()
                    .position(|line| line == "    artifact: result")
                    .unwrap()
                    + 1
            );
            assert_eq!(diagnostic.message, lua_diagnosis.diagnostics[0].message);
            let diagnostic = &lua_diagnosis.diagnostics[0];
            assert_eq!(diagnostic.field.as_deref(), Some("artifact"));
            assert_eq!(diagnostic.node_name.as_deref(), Some("main"));
            assert_eq!(
                diagnostic.span,
                Some(DiagnosticSpan {
                    source: Some(format!("{name}.lua")),
                    start_line: 5,
                    start_col: 5,
                    end_line: 5,
                    end_col: 13,
                })
            );
        }
        for (extension, source) in [("yml", yaml), ("lua", lua.as_str())] {
            let path = tmp.path().join(format!("{name}.{extension}"));
            std::fs::write(&path, source).unwrap();
            let error =
                crate::adaptor::gateway::workflow::storage::load_workflow(&path, tmp.path())
                    .unwrap_err();
            assert!(
                matches!(error,
                    crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(ref items)
                        if items.iter().any(|item| item.code == code && item.stage == DiagnosticStage::ParseShape)
                ),
                "{error:?}"
            );
        }
    }
}

#[test]
pub fn test_sequence多段参照の診断_実loaderが統合mapの参照を受理する() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sequence-merged-references.yml");
    std::fs::write(&path, MERGED_REFERENCES).unwrap();

    // When
    let diagnosis = diagnose_workflow_source(MERGED_REFERENCES, None);
    let loaded = crate::adaptor::gateway::workflow::storage::load_workflow(&path, tmp.path());

    // Then
    assert!(
        diagnosis.diagnostics.is_empty(),
        "{:?}",
        diagnosis.diagnostics
    );
    assert!(loaded.is_ok(), "{loaded:?}");
}

const FANOUT_REFERENCES: &str = include_str!(
    "../../../../../../src/adaptor/gateway/workflow/fixtures/valid/fanout-map-references.yml"
);
const FANOUT_LUA_REFERENCES: &str = r#"local r = require('releash')
local result = r.schema.object{ name = 'result', properties = {
  passed = r.schema.boolean(), tasks = r.schema.array{ items = r.schema.string{} },
}, required = { 'passed', 'tasks' } }
local a = r.command{ name = 'a', command = 'collect', artifact = result }
local fan = r.fanout{ name = 'fan', children = { r.child{ node = a } } }
local indexed_worker = r.command{ name = 'indexed_worker', command = 'work', input = { r.input('item') }, artifact = result }
local indexed = r.fanout{ name = 'indexed', items = fan.a.tasks, children = { r.child{ node = indexed_worker } } }
local nested_a = r.command{ name = 'nested_a', command = 'collect', artifact = result }
local nested_fan = r.fanout{ name = 'nested_fan', children = { r.child{ node = nested_a } } }
local seq = r.sequence{ name = 'seq', children = { r.child{ node = nested_fan } } }
local consume = r.command{ name = 'consume', command = 'consume', input = { r.input('all'), r.input('slot'), r.input('named'), r.input('indexed'), r.input('nested') } }
local worker = r.command{ name = 'worker', command = 'work', input = { r.input('item') } }
local expand = r.fanout{ name = 'expand', items = seq.nested_fan.nested_a.tasks, children = { r.child{ node = worker } } }
return r.workflow{ name = 'fanout-map-references', description = 'Fanout references', main = r.sequence{ children = {
  r.child{ node = fan }, r.child{ node = indexed }, r.child{ node = seq },
  r.child{ node = consume, inputs = { all = fan, slot = fan.a, named = fan.a.passed, indexed = indexed['0'].passed, nested = seq.nested_fan.nested_a.passed } },
  r.child{ node = expand },
} } }
"#;

#[test]
pub fn test_fanout多段参照の診断_yamlとluaの配線とitemsを診断ゼロでloadする() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    for (extension, source) in [("yml", FANOUT_REFERENCES), ("lua", FANOUT_LUA_REFERENCES)] {
        let filename = format!("fanout-map-references.{extension}");
        let path = tmp.path().join(&filename);
        std::fs::write(&path, source).unwrap();

        // When
        let diagnosis = if extension == "yml" {
            diagnose_workflow_source(source, None)
        } else {
            diagnose_lua_workflow_source(&filename, source, tmp.path(), tmp.path(), None)
        };
        let loaded = crate::adaptor::gateway::workflow::storage::load_workflow(&path, tmp.path());

        // Then
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{extension}: {:?}",
            diagnosis.diagnostics
        );
        assert!(loaded.is_ok(), "{loaded:?}");
    }
}

const FANOUT_ROUTING: &str = include_str!(
    "../../../../../../src/adaptor/gateway/workflow/fixtures/valid/fanout-map-routing.yml"
);

#[test]
pub fn test_fanoutの判別規則の診断_yamlとluaのwhenとswitchと入れ子を診断ゼロでloadする() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let lua = r#"local r = require('releash')
local result = r.schema.object{ name = 'result', properties = {
  passed = r.schema.boolean(), verdict = r.schema.string{ enum = { 'READY', 'HOLD' } },
}, required = { 'passed', 'verdict' } }
local a = r.command{ name = 'a', command = 'collect', artifact = result }
local fan = r.fanout{ name = 'fan', children = { r.child{ node = a } } }
local indexed_worker = r.command{ name = 'indexed_worker', command = 'collect', input = { r.input('item') }, artifact = result }
local indexed = r.fanout{ name = 'indexed', items = { 'task' }, children = { r.child{ node = indexed_worker } } }
local classify = r.command{ name = 'classify', command = 'collect', artifact = result }
local classifier = r.fanout{ name = 'classifier', children = { r.child{ node = classify } } }
local nested_a = r.command{ name = 'nested_a', command = 'collect', artifact = result }
local nested_fan = r.fanout{ name = 'nested_fan', children = { r.child{ node = nested_a } } }
local seq = r.sequence{ name = 'seq', children = { r.child{ node = nested_fan } } }
local ready = r.command{ name = 'ready', command = 'ready' }
local finished = r.command{ name = 'finished', command = 'finished' }
return r.workflow{ name = 'fanout-map-routing', description = 'Fanout routing', main = r.sequence{ children = {
  r.child{ node = fan, rules = { r.when{ on = fan.a.passed, on_true = indexed, next = finished } } },
  r.child{ node = indexed, rules = { r.switch{ on = indexed['0'].verdict, cases = { READY = classifier, HOLD = finished } } } },
  r.child{ node = classifier, rules = { r.switch{ on = classifier.classify.verdict, cases = { READY = seq, HOLD = finished } } } },
  r.child{ node = seq, rules = { r.when{ on = seq.nested_fan.nested_a.passed, on_true = ready, next = finished } } },
  r.child{ node = ready, rules = {} }, r.child{ node = finished },
} } }
"#;
    for (extension, source) in [("yml", FANOUT_ROUTING), ("lua", lua)] {
        let filename = format!("fanout-map-routing.{extension}");
        let path = tmp.path().join(&filename);
        std::fs::write(&path, source).unwrap();

        // When
        let diagnosis = if extension == "yml" {
            diagnose_workflow_source(source, None)
        } else {
            diagnose_lua_workflow_source(&filename, source, tmp.path(), tmp.path(), None)
        };
        let loaded = crate::adaptor::gateway::workflow::storage::load_workflow(&path, tmp.path());

        // Then
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{extension}: {:?}",
            diagnosis.diagnostics
        );
        assert!(loaded.is_ok(), "{loaded:?}");
    }
}

#[test]
pub fn test_fanout変更後のbuiltin定義_8本すべて診断ゼロでloadする() {
    // Given
    let summaries = builtin::list_builtin_workflows();
    for summary in summaries {
        let source = builtin::builtin_workflow_source(&summary.name).unwrap();

        // When
        let diagnosis = diagnose_workflow_source(source, Some(&summary.name));
        let loaded = builtin::load_builtin_workflow_resolved(&summary.name);

        // Then
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{}: {:?}",
            summary.name,
            diagnosis.diagnostics
        );
        assert!(matches!(loaded, Ok(Some(_))), "{loaded:?}");
    }
}

const PREDICATE_ROUTING: &str = include_str!(
    "../../../../../../src/adaptor/gateway/workflow/fixtures/valid/predicate-routing.yml"
);
const NESTED_PREDICATE: &str = "{and: [passed, {or: [clean, skipped]}]}";

fn predicate_yaml(on: &str) -> String {
    PREDICATE_ROUTING.replace(NESTED_PREDICATE, on)
}

const PREDICATE_LUA: &str = include_str!(
    "../../../../../../src/adaptor/gateway/workflow/fixtures/valid/predicate-routing.lua"
);

#[test]
pub fn test_lua容量の診断_述語再利用の上限超過は位置を保持してloadを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("budget.lua");
    std::fs::write(
        &path,
        r#"local r = require('releash')
local judge = r.command{ command = 'judge' }
local predicate = r.all{ judge.ok }
for i = 1, 30 do predicate = r.all{ predicate, predicate } end
return r.workflow{ name = 'budget', description = 'test', main = judge }
"#,
    )
    .unwrap();
    // When
    let result = crate::adaptor::gateway::workflow::storage::load_workflow(&path, directory.path());
    // Then
    let Err(crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(diagnostics)) =
        result
    else {
        panic!("{result:?}");
    };
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "WFS010");
    assert_eq!(diagnostic.stage, DiagnosticStage::ParseShape);
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "Lua definition exceeded the limit of 100000 builder values"
    );
    assert_eq!(diagnostic.field, None);
    assert_eq!(
        diagnostic.span,
        Some(DiagnosticSpan {
            source: Some("budget.lua".to_string()),
            start_line: 4,
            start_col: 1,
            end_line: 4,
            end_col: 2,
        })
    );
}
const NESTED_LUA: &str = "r.all{ judge.passed, r.any{ judge.clean, judge.skipped } }";

fn predicate_lua(on: &str) -> String {
    PREDICATE_LUA.replace(NESTED_LUA, on)
}

#[test]
pub fn test_lua参照解決の診断_変換不能な値は利用箇所ごとの理由とfieldでloadを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let invalid_predicate = "predicate must be a field reference or an and/or map";
    let mut cases = Vec::new();
    for value in ["true", "42", "'passed'", "{}", "{ value = true }"] {
        cases.push((predicate_lua(value), invalid_predicate, "on"));
        for builder in ["all", "any"] {
            cases.push((
                predicate_lua(&format!("r.{builder}{{ {value} }}")),
                invalid_predicate,
                "predicate element",
            ));
        }
    }
    for value in ["true", "42", "'passed'", "{}", NESTED_LUA] {
        cases.push((
            predicate_lua(value)
                .replace("r.when{", "r.switch{")
                .replace(
                    "on_true = done, next = fix",
                    "cases = { yes = done }, next = fix",
                ),
            "field 'on' must be Source",
            "on",
        ));
        cases.push((
            PREDICATE_LUA.replace(
                "node = judge, rules = {",
                &format!("node = judge, inputs = {{ data = {value} }}, rules = {{"),
            ),
            "field 'inputs' must be Source values",
            "inputs",
        ));
    }
    for (source, message, field) in cases {
        let path = directory.path().join("sources.lua");
        std::fs::write(&path, &source).unwrap();
        // When
        let result =
            crate::adaptor::gateway::workflow::storage::load_workflow(&path, directory.path());
        // Then
        let Err(crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(diagnostics)) =
            result
        else {
            panic!("{source}: {result:?}");
        };
        assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, "WFS002", "{source}");
        assert_eq!(diagnostic.stage, DiagnosticStage::ParseShape);
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.message, message, "{source}");
        assert_eq!(diagnostic.field.as_deref(), Some(field));
        let line = if field == "inputs" { 22 } else { 23 };
        assert_eq!(
            diagnostic.span,
            Some(DiagnosticSpan {
                source: Some("sources.lua".to_string()),
                start_line: line,
                start_col: 1,
                end_line: line,
                end_col: 2,
            }),
            "{source}"
        );
    }
}

#[test]
pub fn test_lua述語の診断_自childのartifact_field以外はresolveでloadを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for reference in [
        "done.ok",
        "judge",
        "r.request",
        "r.items",
        "r.input('value')",
    ] {
        for expression in [
            reference.to_string(),
            format!("r.all{{ judge.passed, r.any{{ {reference} }} }}"),
        ] {
            let path = directory.path().join("scope.lua");
            std::fs::write(&path, predicate_lua(&expression)).unwrap();
            // When
            let result =
                crate::adaptor::gateway::workflow::storage::load_workflow(&path, directory.path());
            // Then
            let Err(crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(
                diagnostics,
            )) = result
            else {
                panic!("{expression}: {result:?}");
            };
            assert_eq!(diagnostics.len(), 1, "{expression}: {diagnostics:?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.code, "WFR003", "{expression}");
            assert_eq!(diagnostic.stage, DiagnosticStage::Resolve);
            assert_eq!(diagnostic.severity, Severity::Error);
            assert_eq!(
                diagnostic.message,
                "rule discriminator must reference the current child artifact field"
            );
            assert_eq!(diagnostic.field, None);
        }
    }
}

#[test]
pub fn test_述語の表面間同値性_受理と全真理値の遷移が一致する() {
    use crate::domain::workflow::services::routing::{route_in_scope, RouteDecision};
    // Given
    let directory = tempfile::tempdir().unwrap();
    for (yaml_on, lua_on) in [
        ("passed", "judge.passed"),
        ("{and: [passed]}", "r.all{ judge.passed }"),
        ("{or: [passed]}", "r.any{ judge.passed }"),
        (
            "{and: [passed, clean]}",
            "r.all{ judge.passed, judge.clean }",
        ),
        (
            "{or: [passed, clean]}",
            "r.any{ judge.passed, judge.clean }",
        ),
        (NESTED_PREDICATE, NESTED_LUA),
        (
            "{and: [details.passed, {or: [details.passed, passed]}]}",
            "r.all{ judge.details.passed, r.any{ judge.details.passed, judge.passed } }",
        ),
        ("legacy flag", "judge['legacy flag']"),
    ] {
        let yaml = diagnose_workflow_source(&predicate_yaml(yaml_on), None);
        let lua = diagnose_lua_workflow_source(
            "predicate-routing.lua",
            &predicate_lua(lua_on),
            directory.path(),
            directory.path(),
            None,
        );
        // When / Then
        for diagnosis in [&yaml, &lua] {
            assert!(
                diagnosis.diagnostics.is_empty(),
                "{yaml_on}: {:?}",
                diagnosis.diagnostics
            );
            assert!(diagnosis.workflow.is_some());
        }
        for passed in [false, true] {
            for clean in [false, true] {
                for skipped in [false, true] {
                    let expected = match yaml_on {
                        "passed" | "{and: [passed]}" | "{or: [passed]}" | "legacy flag" => passed,
                        "{and: [passed, clean]}" => passed && clean,
                        "{or: [passed, clean]}" => passed || clean,
                        NESTED_PREDICATE => passed && (clean || skipped),
                        _ => passed,
                    };
                    let value = serde_json::json!({"passed": passed, "clean": clean, "skipped": skipped, "details": {"passed": passed}, "legacy flag": passed});
                    for diagnosis in [&yaml, &lua] {
                        let workflow = diagnosis.workflow.as_ref().unwrap();
                        let sequence = workflow.entry_node().unwrap().sequence().unwrap();
                        let target = route_in_scope(
                            workflow,
                            sequence,
                            "judge",
                            Some(&value),
                            &HashMap::new(),
                        )
                        .unwrap();
                        assert_eq!(
                            target,
                            RouteDecision::TransitionTo(
                                if expected { "done" } else { "fix" }.to_string()
                            ),
                            "{yaml_on}: {value}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
pub fn test_述語の表面間同値性_空と型とrequiredとpathの診断が一致する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for (yaml_on, lua_on) in [
        ("{and: []}", "r.all{}"),
        ("{or: []}", "r.any{}"),
        ("{or: [passed, {and: []}]}", "r.any{ judge.passed, r.all{} }"),
        ("{and: [passed, {or: []}]}", "r.all{ judge.passed, r.any{} }"),
        ("details.text", "judge.details.text"),
        ("{or: [passed, details.text]}", "r.any{ judge.passed, judge.details.text }"),
        ("{and: [passed, {or: [details.optional]}]}", "r.all{ judge.passed, r.any{ judge.details.optional } }"),
        ("{or: [passed, details.unknown]}", "r.any{ judge.passed, judge.details.unknown }"),
        ("{or: [passed, passed.flag]}", "r.any{ judge.passed, judge.passed.flag }"),
        ("{or: [passed, {and: [details.text, details.optional, details.unknown]}]}", "r.any{ judge.passed, r.all{ judge.details.text, judge.details.optional, judge.details.unknown } }"),
    ] {
        let yaml = diagnose_workflow_source(&predicate_yaml(yaml_on), None);
        let lua = diagnose_lua_workflow_source("predicate-routing.lua", &predicate_lua(lua_on), directory.path(), directory.path(), None);
        // When / Then
        assert!(yaml.has_errors(), "{yaml_on}");
        assert!(lua.has_errors(), "{lua_on}");
        let signature = |diagnosis: &WorkflowSourceDiagnostics| diagnosis.diagnostics.iter().map(|diagnostic| (diagnostic.code.clone(), diagnostic.stage, diagnostic.message.clone())).collect::<Vec<_>>();
        assert_eq!(signature(&yaml), signature(&lua), "{yaml_on} / {lua_on}");
    }
}

#[test]
pub fn test_述語の表面間同値性_空と不正な要素と配列以外は同じ診断でloadを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let invalid_predicate = "predicate must be a field reference or an and/or map";
    let expected_array = "predicate and/or must contain an array";
    let empty = "predicate and/or must contain at least one element";
    for (yaml_on, lua_on, message) in [
        ("{and: []}", "r.all{}", empty),
        ("{or: []}", "r.any{}", empty),
        (
            "{or: [passed, {and: []}]}",
            "r.any{ judge.passed, r.all{} }",
            empty,
        ),
        (
            "{and: [passed, {or: []}]}",
            "r.all{ judge.passed, r.any{} }",
            empty,
        ),
        ("true", "true", invalid_predicate),
        ("42", "42", invalid_predicate),
        ("[passed]", "{ judge.passed }", invalid_predicate),
        ("{and: [true]}", "r.all{ true }", invalid_predicate),
        (
            "{and: [passed, false]}",
            "r.all{ judge.passed, false }",
            invalid_predicate,
        ),
        (
            "{or: [passed, false]}",
            "r.any{ judge.passed, false }",
            invalid_predicate,
        ),
        ("{and: [42]}", "r.all{ 42 }", invalid_predicate),
        ("{or: [42]}", "r.any{ 42 }", invalid_predicate),
        (
            "{and: [passed, {or: [false]}]}",
            "r.all{ judge.passed, r.any{ false } }",
            invalid_predicate,
        ),
        (
            "{or: [passed, {and: [false]}]}",
            "r.any{ judge.passed, r.all{ false } }",
            invalid_predicate,
        ),
        (
            "{and: {field: passed}}",
            "r.all{ field = judge.passed }",
            expected_array,
        ),
        (
            "{or: {field: passed}}",
            "r.any{ field = judge.passed }",
            expected_array,
        ),
        ("{and: true}", "r.all(true)", expected_array),
        ("{or: passed}", "r.any('passed')", expected_array),
        ("{and: null}", "r.all(nil)", expected_array),
        ("{or: 42}", "r.any(42)", expected_array),
        (
            "{and: [passed, {or: {field: passed}}]}",
            "r.all{ judge.passed, r.any{ field = judge.passed } }",
            expected_array,
        ),
        (
            "{or: [passed, {and: false}]}",
            "r.any{ judge.passed, r.all(false) }",
            expected_array,
        ),
    ] {
        for (extension, source) in [
            ("yml", predicate_yaml(yaml_on)),
            ("lua", predicate_lua(lua_on)),
        ] {
            let path = directory
                .path()
                .join(format!("predicate-routing.{extension}"));
            std::fs::write(&path, source).unwrap();
            // When
            let result =
                crate::adaptor::gateway::workflow::storage::load_workflow(&path, directory.path());
            // Then
            let Err(crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(
                diagnostics,
            )) = result
            else {
                panic!("{extension}: {yaml_on} / {lua_on}: {result:?}");
            };
            let signature: Vec<_> = diagnostics
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.code.as_str(),
                        diagnostic.stage,
                        diagnostic.severity,
                        diagnostic.message.as_str(),
                    )
                })
                .collect();
            assert_eq!(
                signature,
                vec![(
                    "WFS002",
                    DiagnosticStage::ParseShape,
                    Severity::Error,
                    message
                )],
                "{extension}: {yaml_on} / {lua_on}",
            );
        }
    }
}

#[test]
pub fn test_述語の表面間同値性_sequenceとfanoutの異なるslotを合成する() {
    use crate::domain::workflow::services::routing::{route_in_scope, RouteDecision};
    // Given
    let directory = tempfile::tempdir().unwrap();
    for (yaml_node, lua_node, fields) in [
        ("sequence: {children: [a, b]}", "r.sequence{ name = 'judge', children = { r.child{node = a}, r.child{node = b} } }", ["a.details.passed", "b.passed"]),
        ("fanout: {children: [a, b]}", "r.fanout{ name = 'judge', children = { r.child{node = a}, r.child{node = b} } }", ["a.details.passed", "b.passed"]),
        ("fanout: {children: [a], items: [first, second]}", "r.fanout{ name = 'judge', children = { r.child{node = a} }, items = {'first', 'second'} }", ["0.details.passed", "1.passed"]),
        ("sequence: {children: [fan]}", "r.sequence{ name = 'judge', children = { r.child{node = fan} } }", ["fan.a.details.passed", "fan.b.passed"]),
    ] {
        let indexed = fields[0].starts_with('0');
        let nested = fields[0].starts_with("fan.");
        let yaml = predicate_yaml(&format!("{{and: [{}, {{or: [{}]}}]}}", fields[0], fields[1]))
            .replace("  judge:\n    command: judge\n    artifact: result", &format!("  judge:\n    {yaml_node}\n  a: {{command: a, artifact: result{}}}{}{}", if indexed {", input: [item]"} else {""}, if indexed {""} else {"\n  b: {command: b, artifact: result}"}, if nested {"\n  fan: {fanout: {children: [a, b]}}"} else {""}));
        let lua_reference = |field: &str| field.split('.').fold("judge".to_string(), |source, segment| format!("{source}['{segment}']"));
        let lua = predicate_lua(&format!("r.all{{ {}, r.any{{ {} }} }}", lua_reference(fields[0]), lua_reference(fields[1])))
            .replace("local judge = r.command{ name = \"judge\", command = \"judge\", artifact = result }", &format!("local a = r.command{{name = 'a', command = 'a', artifact = result{}}}\n{}{}local judge = {lua_node}", if indexed {", input = { r.input('item') }"} else {""}, if indexed {""} else {"local b = r.command{name = 'b', command = 'b', artifact = result}\n"}, if nested {"local fan = r.fanout{name = 'fan', children = { r.child{node = a}, r.child{node = b} }}\n"} else {""}));
        for (extension, source) in [("yml", yaml), ("lua", lua)] {
            let path = directory.path().join(format!("predicate-routing.{extension}"));
            std::fs::write(&path, source).unwrap();
            // When
            let workflow = crate::adaptor::gateway::workflow::storage::load_workflow(&path, directory.path()).unwrap();
            let sequence = workflow.entry_node().unwrap().sequence().unwrap();
            // Then
            for first in [false, true] {
                for second in [false, true] {
                    let mut artifact = serde_json::json!({});
                    for (field, value) in fields.iter().zip([first, second]) {
                        let mut cursor = &mut artifact;
                        let parts: Vec<_> = field.split('.').collect();
                        for segment in &parts[..parts.len()-1] {
                            if cursor.get(*segment).is_none() { cursor[*segment] = serde_json::json!({}); }
                            cursor = &mut cursor[*segment];
                        }
                        cursor[parts[parts.len()-1]] = serde_json::json!(value);
                    }
                    let decision = route_in_scope(&workflow, sequence, "judge", Some(&artifact), &HashMap::new()).unwrap();
                    assert_eq!(decision, RouteDecision::TransitionTo(if first && second {"done"} else {"fix"}.to_string()), "{extension}: {artifact}");
                }
            }
        }
    }
}

#[test]
pub fn test_述語の実loader_不正なshapeと参照を両表面でloadしない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for (yaml_on, lua_on) in [
        ("{and: []}", "r.all{}"),
        (
            "{or: [passed, details.text]}",
            "r.any{judge.passed, judge.details.text}",
        ),
        ("{and: [details.optional]}", "r.all{judge.details.optional}"),
        ("{or: [details.unknown]}", "r.any{judge.details.unknown}"),
        ("{and: [passed.flag]}", "r.all{judge.passed.flag}"),
    ] {
        for (extension, source) in [
            ("yml", predicate_yaml(yaml_on)),
            ("lua", predicate_lua(lua_on)),
        ] {
            let path = directory
                .path()
                .join(format!("predicate-routing.{extension}"));
            std::fs::write(&path, source).unwrap();
            // When
            let result =
                crate::adaptor::gateway::workflow::storage::load_workflow(&path, directory.path());
            // Then
            assert!(
                matches!(
                    result,
                    Err(crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(_))
                ),
                "{result:?}"
            );
        }
    }
}

#[test]
pub fn test_completion診断_yamlとluaの同じ誤りはcode_stageと各表面のmessageを保つ() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for (yaml_value, lua_value, message) in [
        (
            "{require: approval, extra: true}",
            "{ require = r.completion.approval, extra = true }",
            "completion map contains an unsupported key",
        ),
        (
            "approval",
            "r.completion.approval",
            "completion must be a map",
        ),
        ("auto", "'auto'", "completion must be a map"),
        ("approval", "'approval'", "completion must be a map"),
        ("true", "true", "completion must be a map"),
        ("42", "42", "completion must be a map"),
        (
            "[approval]",
            "{ r.completion.approval }",
            "completion must be a map",
        ),
        (
            "{}",
            "{}",
            "completion must contain at least one requirement",
        ),
        (
            "{require: auto}",
            "{ require = 'auto' }",
            "completion require must be approval",
        ),
        (
            "{require: other}",
            "{ require = 'other' }",
            "completion require must be approval",
        ),
        (
            "{require: true}",
            "{ require = true }",
            "completion require must be approval",
        ),
        (
            "{require: 1}",
            "{ require = 1 }",
            "completion require must be approval",
        ),
        (
            "{require: {}}",
            "{ require = {} }",
            "completion require must be approval",
        ),
        (
            "{require: []}",
            "{ require = { r.completion.approval } }",
            "completion require must be approval",
        ),
    ] {
        let lua_source = format!("local r = require('releash')\nreturn r.workflow{{ name = 'completion', description = 'test', main = r.command{{\n  command = 'true',\n  completion = {lua_value},\n}} }}");
        let yaml_bodies = [
            format!("  main:\n    command: 'true'\n    completion: {yaml_value}"),
            format!("  main:\n    sequence:\n      children:\n        - leaf:\n            command: 'true'\n            completion: {yaml_value}"),
            format!("  main:\n    fanout:\n      children:\n        - command: 'true'\n          completion: {yaml_value}"),
        ];
        for body in yaml_bodies {
            let yaml_source = format!("name: completion\ndescription: test\nnodes:\n{body}\n");
            // When
            let yaml = diagnose_workflow_source(&yaml_source, None);
            let lua = diagnose_lua_workflow_source(
                "completion.lua",
                &lua_source,
                directory.path(),
                directory.path(),
                None,
            );
            // Then
            let lua_message = match message {
                "completion map contains an unsupported key" => {
                    "completion map only accepts the key 'require'"
                }
                _ => message,
            };
            for (diagnosis, message) in [(&yaml, message), (&lua, lua_message)] {
                assert!(diagnosis.workflow.is_none(), "{yaml_value} / {lua_value}");
                assert_eq!(
                    diagnosis.diagnostics.len(),
                    1,
                    "{:?}",
                    diagnosis.diagnostics
                );
                let diagnostic = &diagnosis.diagnostics[0];
                assert_eq!(diagnostic.code, "WFS002");
                assert_eq!(diagnostic.stage, DiagnosticStage::ParseShape);
                assert_eq!(diagnostic.severity, Severity::Error);
                assert_eq!(diagnostic.message, message);
                assert_eq!(diagnostic.field.as_deref(), Some("completion"));
                assert!(diagnostic.span.is_some());
            }
        }
    }
}

#[test]
pub fn test_completion診断_全node種別でyamlとluaが同じ要求の有無を持つ定義を構築する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("instructions")).unwrap();
    std::fs::write(
        directory.path().join("instructions/test.md"),
        "test instruction",
    )
    .unwrap();
    for (yaml_kind, lua_kind) in [
        ("session: {provider: claude, facets: {instruction: test}}", "r.session{ provider = r.provider.claude, facets = {instruction = f.instruction.test}, %COMPLETION% }"),
        ("command: 'true'", "r.command{ command = 'true', %COMPLETION% }"),
        ("fanout: {children: [{leaf: {command: 'true'}}]}", "r.fanout{ children = {r.child{node = r.command{name = 'leaf', command = 'true'}}}, %COMPLETION% }"),
        ("sequence: {children: [{leaf: {command: 'true'}}]}", "r.sequence{ children = {r.child{node = r.command{name = 'leaf', command = 'true'}}}, %COMPLETION% }"),
    ] {
        for required in [false, true] {
            let yaml_completion = if required { "\n    completion: {require: approval}" } else { "" };
            let lua_completion = if required { "completion = { require = r.completion.approval }" } else { "" };
            let yaml_source = format!("name: completion\ndescription: test\nnodes:\n  main:\n    {yaml_kind}{yaml_completion}\n");
            let lua_source = format!("local r = require('releash')\nlocal f = require('facets')\nreturn r.workflow{{ name = 'completion', description = 'test', main = {} }}", lua_kind.replace("%COMPLETION%", lua_completion));
            // When
            let yaml = diagnose_workflow_source(&yaml_source, None);
            let lua = diagnose_lua_workflow_source("completion.lua", &lua_source, directory.path(), directory.path(), None);
            // Then
            assert!(yaml.diagnostics.is_empty(), "{:?}", yaml.diagnostics);
            assert!(lua.diagnostics.is_empty(), "{:?}", lua.diagnostics);
            let yaml_workflow = yaml.workflow.unwrap();
            let lua_workflow = lua.workflow.unwrap();
            assert_eq!(serde_json::to_value(&yaml_workflow).unwrap(), serde_json::to_value(&lua_workflow).unwrap());
            assert_eq!(yaml_workflow.node_by_name("main").unwrap().requires_approval_completion(), required);
        }
    }
}

#[test]
pub fn test_隔離定義_yamlとluaの全node種別でmodeを受理する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("instructions")).unwrap();
    std::fs::write(
        directory.path().join("instructions/test.md"),
        "test instruction",
    )
    .unwrap();
    for mode in ["shared", "isolated"] {
        let yaml = format!("name: isolation\ndescription: test\nnodes:\n  main: {{worktree: {mode}, sequence: {{children: [group]}}}}\n  group: {{worktree: {mode}, fanout: {{children: [agent, check]}}}}\n  agent: {{worktree: {mode}, session: {{provider: codex, facets: {{instruction: test}}}}}}\n  check: {{worktree: {mode}, command: 'true'}}");
        let lua = format!(
            r#"local r = require('releash')
local f = require('facets')
local agent = r.session{{name = 'agent', provider = r.provider.codex, facets = {{instruction = f.instruction.test}}, worktree = r.worktree.{mode}}}
local check = r.command{{name = 'check', command = 'true', worktree = r.worktree.{mode}}}
local group = r.fanout{{name = 'group', worktree = r.worktree.{mode}, children = {{r.child{{node = agent}}, r.child{{node = check}}}}}}
return r.workflow{{name = 'isolation', description = 'test', main = r.sequence{{worktree = r.worktree.{mode}, children = {{r.child{{node = group}}}}}}}}
"#
        );

        // When
        let diagnoses = [
            diagnose_workflow_source(&yaml, None),
            diagnose_lua_workflow_source(
                "isolation.lua",
                &lua,
                directory.path(),
                directory.path(),
                None,
            ),
        ];

        // Then
        for diagnosis in diagnoses {
            assert!(
                diagnosis.diagnostics.is_empty(),
                "{:?}",
                diagnosis.diagnostics
            );
            let workflow = diagnosis.workflow.unwrap();
            assert_eq!(workflow.nodes.len(), 4);
            assert!(workflow
                .nodes
                .iter()
                .all(|node| node.is_isolated() == (mode == "isolated")));
        }
    }
}

#[test]
pub fn test_隔離定義_値域外のyaml値とluaの文字列や他のhandleを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("instructions")).unwrap();
    std::fs::write(
        directory.path().join("instructions/test.md"),
        "test instruction",
    )
    .unwrap();
    for value in ["unknown", "42", "true", "[]", "{}", "null"] {
        let source = format!("name: invalid\ndescription: test\nnodes:\n  main: {{command: 'true', worktree: {value}}}");

        // When
        let diagnosis = diagnose_workflow_source(&source, None);

        // Then
        assert!(diagnosis.workflow.is_none(), "{value}");
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error),
            "{value}"
        );
    }
    for value in ["'isolated'", "42", "true", "{}", "r.provider.codex"] {
        let source = format!("local r = require('releash')\nreturn r.workflow{{name = 'invalid', description = 'test', main = r.command{{command = 'true', worktree = {value}}}}}");
        let diagnosis = diagnose_lua_workflow_source(
            "invalid.lua",
            &source,
            directory.path(),
            directory.path(),
            None,
        );
        assert!(diagnosis.workflow.is_none(), "{value}");
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error),
            "{value}"
        );
    }
}

#[test]
pub fn test_隔離定義_合成子とcontractなしsessionを経由してworktreeを参照する() {
    // Given
    let yaml = "name: references\ndescription: test\nnodes:\n  main:\n    sequence:\n      children:\n        - seq\n        - report: {inputs: {path: seq.work.worktree.path, branch: seq.worktree.branch}}\n  seq: {worktree: isolated, sequence: {children: [work]}}\n  work: {worktree: isolated, session: {provider: codex, facets: {instruction: test}}}\n  report: {input: [path, branch], command: 'echo {{ path }}', env: {BRANCH: branch}}";
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("instructions")).unwrap();
    std::fs::write(
        directory.path().join("instructions/test.md"),
        "test instruction",
    )
    .unwrap();
    let lua = r#"local r = require('releash')
local f = require('facets')
local work = r.session{name = 'work', provider = r.provider.codex, facets = {instruction = f.instruction.test}, worktree = r.worktree.isolated}
local seq = r.sequence{name = 'seq', worktree = r.worktree.isolated, children = {r.child{node = work}}}
local path = seq.work.worktree.path
local branch = seq.worktree.branch
return r.workflow{name = 'references', description = 'test', main = r.sequence{children = {r.child{node = seq}}}}
"#;

    // When
    let diagnoses = [
        diagnose_workflow_source(yaml, None),
        diagnose_lua_workflow_source(
            "references.lua",
            lua,
            directory.path(),
            directory.path(),
            None,
        ),
    ];

    // Then
    for diagnosis in diagnoses {
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{:?}",
            diagnosis.diagnostics
        );
        assert!(diagnosis.workflow.is_some());
    }
}
pub(crate) mod tests {
use crate::adaptor::gateway::workflow::facet;

    use super::super::*;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, FacetRefs, FanoutSpec, ItemsSource, NodeKind, Rule, SchemaDef, SessionSpec,
        WorkflowDefinitionYaml,
    };
    use crate::domain::workflow::InputParam;
    use std::fs;
    use tempfile::TempDir;

    fn make_node(name: &str, instruction: Option<&str>) -> NodeDefinition {
        let facets = FacetRefs {
            instruction: instruction.map(str::to_string),
            ..Default::default()
        };
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets,
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_session_node(
        name: &str,
        policy: Option<&str>,
        knowledge: &[&str],
        instruction: Option<&str>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    policy: policy.map(str::to_string),
                    knowledge: knowledge.iter().map(|key| (*key).to_string()).collect(),
                    instruction: instruction.map(str::to_string),
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_child(name: &str, instruction: Option<&str>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: instruction.map(str::to_string),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_fanout(name: &str, children: Vec<&str>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                children: children
                    .into_iter()
                    .map(crate::domain::workflow::ChildEntry::reference)
                    .collect(),
                items: None,
            }),
            ..NodeDefinition::default()
        }
    }

    fn make_command(name: &str, command: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
                env: Default::default(),
            }),
            ..NodeDefinition::default()
        }
    }

    fn setup_facet(dir: &Path, kind: &str, key: &str, content: &str) {
        let facet_dir = dir.join(kind);
        fs::create_dir_all(&facet_dir).unwrap();
        fs::write(facet_dir.join(format!("{key}.md")), content).unwrap();
    }

    #[test]
    pub fn test_診断reportはserializeとdeserializeをround_tripできる() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("round-trip.yml"),
            r#"name: round-trip
description: round trip
nodes:
  main:
    command: printf ok
"#,
        )
        .unwrap();

        // When
        let report = diagnose_all(tmp.path(), tmp.path()).unwrap();
        let value = serde_json::to_value(
            crate::adaptor::presenter::workflow_api::DiagnosticReportResponse::from(report),
        )
        .unwrap();
        let decoded = serde_json::from_value::<
            crate::adaptor::presenter::workflow_api::DiagnosticReportResponse,
        >(value.clone())
        .unwrap();

        // Then
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }

    fn permission_yaml(permission: &str) -> String {
        format!(
            r#"name: permission
description: permission contract
nodes:
  main:
    session:
      provider: claude
      permission: {permission}
      facets:
        instruction: test
"#
        )
    }

    fn permission_lua(permission: &str) -> String {
        format!(
            r#"local r = require("releash")
local f = require("facets")
return r.workflow{{
  name = "permission", description = "permission contract",
  main = r.session{{ provider = r.provider.claude, permission = "{permission}", facets = {{ instruction = f.instruction.test }} }},
}}
"#
        )
    }

    fn permission_lua_value(permission: &str) -> String {
        format!(
            r#"local r = require("releash")
local f = require("facets")
return r.workflow{{
  name = "permission", description = "permission contract",
  main = r.session{{ provider = r.provider.claude, permission = {permission}, facets = {{ instruction = f.instruction.test }} }},
}}
"#
        )
    }

    fn inline_child_permission_yaml(permission: &str) -> String {
        format!(
            r#"name: permission
description: permission contract
nodes:
  main:
    sequence:
      children:
      - session:
          provider: claude
          permission: {permission}
          facets:
            instruction: test
"#
        )
    }

    fn named_inline_child_permission_yaml(permission: &str) -> String {
        format!(
            r#"name: permission
description: permission contract
nodes:
  main:
    sequence:
      children:
      - review:
          session:
            provider: claude
            permission: {permission}
            facets:
              instruction: test
"#
        )
    }

    fn inline_child_permission_lua(permission: &str) -> String {
        format!(
            r#"local r = require("releash")
local f = require("facets")
local child = r.session{{ provider = r.provider.claude, permission = "{permission}", facets = {{ instruction = f.instruction.test }} }}
return r.workflow{{
  name = "permission", description = "permission contract",
  main = r.sequence{{ children = {{ r.child{{ node = child }} }} }},
}}
"#
        )
    }

    #[test]
    pub fn test_診断_yamlとluaのsession_permission_4値をerrorなしで受理する() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        for permission in ["manual", "auto", "bypass", "read-only"] {
            let yaml = diagnose_workflow_source(&permission_yaml(permission), Some("permission"));
            assert!(
                !yaml.has_errors(),
                "YAML {permission} produced diagnostics: {:?}",
                yaml.diagnostics
            );
            let lua = diagnose_lua_workflow_source(
                "permission.lua",
                &permission_lua(permission),
                tmp.path(),
                tmp.path(),
                Some("permission"),
            );
            assert!(
                !lua.has_errors(),
                "Lua {permission} produced diagnostics: {:?}",
                lua.diagnostics
            );
        }
    }

    #[test]
    pub fn test_診断_yamlとluaの不正permissionは同じwfs002でfield位置を示す() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        for invalid in ["unknown", "acceptEdits", "danger-full-access"] {
            let yaml = diagnose_workflow_source(&permission_yaml(invalid), Some("permission"));
            let lua = diagnose_lua_workflow_source(
                "permission.lua",
                &permission_lua(invalid),
                tmp.path(),
                tmp.path(),
                Some("permission"),
            );
            let yaml_item = yaml.diagnostics.first().unwrap();
            let lua_item = lua.diagnostics.first().unwrap();

            assert_eq!(yaml_item.code, "WFS002");
            assert_eq!(lua_item.code, yaml_item.code);
            assert_eq!(yaml_item.stage, DiagnosticStage::ParseShape);
            assert_eq!(lua_item.stage, yaml_item.stage);
            assert_eq!(yaml_item.message, lua_item.message);
            assert_eq!(yaml_item.field.as_deref(), Some("permission"));
            assert_eq!(lua_item.field, yaml_item.field);
            assert_eq!(yaml_item.span.as_ref().unwrap().start_line, 7);
            assert_eq!(lua_item.span.as_ref().unwrap().start_line, 5);
            assert!(yaml.workflow.is_none());
            assert!(lua.workflow.is_none());
        }
    }

    #[test]
    pub fn test_診断_yamlとluaの非文字列permissionは同じwfs002でfield位置を示す() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        let yaml = diagnose_workflow_source(&permission_yaml("5"), Some("permission"));
        let lua = diagnose_lua_workflow_source(
            "permission.lua",
            &permission_lua_value("5"),
            tmp.path(),
            tmp.path(),
            Some("permission"),
        );
        let yaml_item = yaml.diagnostics.first().unwrap();
        let lua_item = lua.diagnostics.first().unwrap();

        assert_eq!(yaml_item.code, "WFS002");
        assert_eq!(lua_item.code, yaml_item.code);
        assert_eq!(yaml_item.stage, DiagnosticStage::ParseShape);
        assert_eq!(lua_item.stage, yaml_item.stage);
        assert_eq!(yaml_item.message, "field 'permission' must be string");
        assert_eq!(lua_item.message, yaml_item.message);
        assert_eq!(yaml_item.field.as_deref(), Some("permission"));
        assert_eq!(lua_item.field, yaml_item.field);
        assert!(yaml.workflow.is_none());
        assert!(lua.workflow.is_none());
    }

    #[test]
    pub fn test_診断_childrenインラインsessionの不正permissionはtop_levelとluaに一致する() {
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "instructions", "test", "test instruction");

        let top_level =
            diagnose_workflow_source(&permission_yaml("acceptEdits"), Some("permission"));
        let inline_child = diagnose_workflow_source(
            &inline_child_permission_yaml("acceptEdits"),
            Some("permission"),
        );
        let named_inline_child = diagnose_workflow_source(
            &named_inline_child_permission_yaml("acceptEdits"),
            Some("permission"),
        );
        let lua = diagnose_lua_workflow_source(
            "permission.lua",
            &inline_child_permission_lua("acceptEdits"),
            tmp.path(),
            tmp.path(),
            Some("permission"),
        );

        let top_level_item = top_level.diagnostics.first().unwrap();
        let inline_child_item = inline_child.diagnostics.first().unwrap();
        let named_inline_child_item = named_inline_child.diagnostics.first().unwrap();
        let lua_item = lua.diagnostics.first().unwrap();
        for item in [inline_child_item, named_inline_child_item, lua_item] {
            assert_eq!(item.code, top_level_item.code);
            assert_eq!(item.stage, top_level_item.stage);
            assert_eq!(item.message, top_level_item.message);
            assert_eq!(item.field, top_level_item.field);
        }
        assert_eq!(inline_child_item.node_name.as_deref(), Some("main"));
        assert_eq!(inline_child_item.span.as_ref().unwrap().start_line, 9);
        assert_eq!(named_inline_child_item.node_name.as_deref(), Some("main"));
        assert_eq!(
            named_inline_child_item.span.as_ref().unwrap().start_line,
            10
        );
        assert_eq!(lua_item.span.as_ref().unwrap().start_line, 3);
        assert!(top_level.workflow.is_none());
        assert!(inline_child.workflow.is_none());
        assert!(named_inline_child.workflow.is_none());
        assert!(lua.workflow.is_none());
    }

    fn save_workflow_yaml(dir: &Path, wf: &WorkflowDefinitionYaml) {
        fs::create_dir_all(dir).unwrap();
        let content = serde_saphyr::to_string(wf).unwrap();
        fs::write(dir.join(format!("{}.yml", wf.name)), content).unwrap();
    }

    #[test]
    pub fn test_診断_指定directory経路はbuiltin_workflowを列挙しない() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("custom.yml"),
            r#"name: custom
description: custom workflow
nodes:
  main:
    command: printf ok
"#,
        )
        .unwrap();
        let builtin_names: HashSet<_> = builtin::list_builtin_workflows()
            .into_iter()
            .map(|summary| summary.name)
            .collect();

        // When
        let directory_report = diagnose_directory(tmp.path()).unwrap();
        let all_report = diagnose_all(tmp.path(), tmp.path()).unwrap();

        // Then
        assert!(directory_report
            .workflow_summaries
            .keys()
            .all(|name| !builtin_names.contains(name)));
        assert!(directory_report.items.iter().all(|item| item
            .workflow_name
            .as_ref()
            .is_none_or(|name| !builtin_names.contains(name))));

        assert!(all_report
            .workflow_summaries
            .keys()
            .any(|name| builtin_names.contains(name)));
        assert!(all_report.items.iter().any(|item| item
            .workflow_name
            .as_ref()
            .is_some_and(|name| builtin_names.contains(name))));
    }

    #[test]
    pub fn test_診断_指定directory経路は参照されないfacetを列挙しない() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "policies", "unused", "unused policy");
        fs::write(
            tmp.path().join("custom.yml"),
            r#"name: custom
description: custom workflow
nodes:
  main:
    command: printf ok
"#,
        )
        .unwrap();

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(!report.facet_summaries.contains_key("policy/unused"));
        assert!(report
            .items
            .iter()
            .all(|item| item.facet_key.as_deref() != Some("unused")));
    }

    #[test]
    pub fn test_診断_指定directory経路はfacets配下の参照済みcustom_facetを判定対象に含める() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "custom-instruction",
            "custom instruction",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "canonical-custom-facet".to_string(),
            description: "canonical custom facet diagnostic".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("custom-instruction"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(!report.items.iter().any(|item| {
            item.code == "FAC002" && item.facet_key.as_deref() == Some("custom-instruction")
        }));
        assert!(report
            .facet_summaries
            .contains_key("instruction/custom-instruction"));
        assert!(report
            .facet_usage
            .contains_key("instruction/custom-instruction"));
    }

    #[test]
    pub fn test_診断_指定directory経路はfacets配下のdisk本文をbuiltinより優先する() {
        // Given
        let tmp = TempDir::new().unwrap();
        let builtin_instruction =
            builtin::list_builtin_facet_keys(FacetKind::Instruction)[0].to_string();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            &builtin_instruction,
            "{{ bad ref }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "builtin-override".to_string(),
            description: "disk facet overrides builtin".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some(&builtin_instruction))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some(&builtin_instruction)
        }));
    }

    #[test]
    pub fn test_診断_指定directory経路は不正参照があっても他のcustomとbuiltin_facetを保持する() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "custom-instruction",
            "{{ bad ref }}",
        );
        let builtin_knowledge =
            builtin::list_builtin_facet_keys(FacetKind::Knowledge)[0].to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "mixed-references".to_string(),
            description: "mixed valid and invalid references".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("Bad.Key"),
                &[&builtin_knowledge],
                Some("custom-instruction"),
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(report
            .items
            .iter()
            .any(|item| { item.code == "FAC002" && item.facet_key.as_deref() == Some("Bad.Key") }));
        for valid_key in [&builtin_knowledge, "custom-instruction"] {
            assert!(!report.items.iter().any(|item| {
                item.code == "FAC002" && item.facet_key.as_deref() == Some(valid_key)
            }));
        }
        assert!(report
            .facet_summaries
            .contains_key("instruction/custom-instruction"));
        assert!(report
            .facet_usage
            .contains_key("instruction/custom-instruction"));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("custom-instruction")
        }));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC000" && item.facet_key.as_deref() == Some(&builtin_knowledge)
        }));
    }

    #[test]
    pub fn test_診断_指定directory経路は一部inventoryのio失敗を返す() {
        // Given
        let tmp = TempDir::new().unwrap();
        let facets_dir = tmp.path().join("facets");
        fs::create_dir_all(&facets_dir).unwrap();
        fs::write(facets_dir.join("policies"), "not a directory").unwrap();
        setup_facet(&facets_dir, "knowledge", "known", "{{ bad ref }}");
        let workflow = WorkflowDefinitionYaml {
            name: "broken-inventory".to_string(),
            description: "one broken facet inventory".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("custom-policy"),
                &["known"],
                None,
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);
        // When
        let result = diagnose_directory(tmp.path());
        // Then
        assert!(matches!(
            result,
            Err(crate::adaptor::gateway::workflow::storage::StorageError::FacetResolution(_))
        ));
    }

    #[test]
    pub fn test_診断_指定directory経路は不正keyと同居するknowledge_facetを保持する() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(tmp.path(), "knowledge", "team-guide", "{{ bad ref }}");
        let workflow = WorkflowDefinitionYaml {
            name: "mixed-knowledge".to_string(),
            description: "valid knowledge and invalid instruction".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                None,
                &["team-guide"],
                Some("setup.v2"),
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        let missing_refs = report
            .items
            .iter()
            .filter(|item| item.code == "FAC002")
            .collect::<Vec<_>>();
        assert_eq!(missing_refs.len(), 1);
        assert_eq!(missing_refs[0].facet_key.as_deref(), Some("setup.v2"));
        assert!(report.facet_summaries.contains_key("knowledge/team-guide"));
        assert!(report.facet_usage.contains_key("knowledge/team-guide"));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("team-guide")
        }));
    }

    #[test]
    pub fn test_診断_指定directory経路のfac002集合はall_available経路と一致する() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            tmp.path(),
            "instructions",
            "real-instruction",
            "{{ bad ref }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "scope-parity".to_string(),
            description: "facet reference parity".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("-bad"),
                &[],
                Some("real-instruction"),
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let directory_report = diagnose_directory(tmp.path()).unwrap();
        let all_available_report = diagnose_all(tmp.path(), tmp.path()).unwrap();
        let fac002_keys = |report: &DiagnosticReport| {
            report
                .items
                .iter()
                .filter(|item| item.code == "FAC002")
                .filter_map(|item| item.facet_key.clone())
                .collect::<HashSet<_>>()
        };

        // Then
        assert_eq!(
            fac002_keys(&directory_report),
            fac002_keys(&all_available_report)
        );
        assert_eq!(
            fac002_keys(&directory_report),
            HashSet::from(["-bad".to_string()])
        );
        assert!(directory_report
            .facet_summaries
            .contains_key("instruction/real-instruction"));
        assert!(directory_report
            .facet_usage
            .contains_key("instruction/real-instruction"));
        assert!(directory_report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("real-instruction")
        }));
    }

    #[test]
    pub fn test_診断_指定directory経路は直下inventory破損時は失敗を返す() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        setup_facet(tmp.path(), "knowledge", "known", "{{ bad ref }}");
        let workflow = WorkflowDefinitionYaml {
            name: "cross-kind-degrade".to_string(),
            description: "broken policy inventory and healthy knowledge".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node(
                "main",
                Some("custom-policy"),
                &["known"],
                None,
            )],
        };
        save_workflow_yaml(tmp.path(), &workflow);
        // When
        let result = diagnose_directory(tmp.path());
        // Then
        assert!(matches!(
            result,
            Err(crate::adaptor::gateway::workflow::storage::StorageError::FacetResolution(_))
        ));
    }

    #[test]
    pub fn test_診断_指定directory経路で実体のない参照facetをfac002にする() {
        // Given
        let tmp = TempDir::new().unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "missing-facet".to_string(),
            description: "missing facet reference".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_session_node("main", Some("missing-policy"), &[], None)],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();
        let item = report
            .items
            .iter()
            .find(|item| {
                item.code == "FAC002" && item.facet_key.as_deref() == Some("missing-policy")
            })
            .expect("missing Facet reference must produce FAC002");

        // Then
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(item.stage, DiagnosticStage::Resolve);
        assert_eq!(item.workflow_name.as_deref(), Some("missing-facet"));
        assert_eq!(item.node_name.as_deref(), Some("main"));
        assert_eq!(item.facet_key.as_deref(), Some("missing-policy"));
        assert_eq!(item.facet_kind.as_deref(), Some("policy"));
        assert_eq!(item.field.as_deref(), Some("policy"));
        assert!(item.message.contains("missing-policy"));
        let usage = report.facet_usage.get("policy/missing-policy").unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].workflow_name, "missing-facet");
        assert_eq!(usage[0].node_name, "main");
        assert_eq!(usage[0].slot, "policy");
    }

    #[test]
    pub fn test_診断_指定directory経路はfacet本文の不正template構文をfac003にする() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "invalid-template",
            "{{ bad ref }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "facet-syntax".to_string(),
            description: "invalid Facet template syntax".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("invalid-template"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(report.items.iter().any(|item| {
            item.code == "FAC003" && item.facet_key.as_deref() == Some("invalid-template")
        }));
        assert!(
            report
                .facet_summaries
                .get("instruction/invalid-template")
                .unwrap()
                .error_count
                > 0
        );
    }

    #[test]
    pub fn test_診断_指定directory経路はworkflow文脈と不整合なfacet本文をwfr003にする() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            &tmp.path().join("facets"),
            "instructions",
            "invalid-reference",
            "Use {{ missing_node }}",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "facet-reference".to_string(),
            description: "invalid Facet template reference".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("invalid-reference"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(report.items.iter().any(|item| {
            item.code == "WFR003"
                && item.workflow_name.as_deref() == Some("facet-reference")
                && item.node_name.as_deref() == Some("main")
                && item.facet_key.as_deref() == Some("invalid-reference")
        }));
        assert!(
            report
                .facet_summaries
                .get("instruction/invalid-reference")
                .unwrap()
                .error_count
                > 0
        );
    }

    #[test]
    pub fn test_診断_指定directory経路は直下の参照済みcustom_facetを判定対象に含める() {
        // Given
        let tmp = TempDir::new().unwrap();
        setup_facet(
            tmp.path(),
            "instructions",
            "custom-instruction",
            "custom instruction",
        );
        let workflow = WorkflowDefinitionYaml {
            name: "custom-facet".to_string(),
            description: "custom facet diagnostic".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("custom-instruction"))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(!report.items.iter().any(|item| {
            item.code == "FAC002" && item.facet_key.as_deref() == Some("custom-instruction")
        }));
        assert!(report
            .facet_summaries
            .contains_key("instruction/custom-instruction"));
        assert!(report
            .facet_usage
            .contains_key("instruction/custom-instruction"));
    }

    #[test]
    pub fn test_診断_指定directory経路は参照済みbuiltin_facetを判定対象に含める() {
        // Given
        let tmp = TempDir::new().unwrap();
        let builtin_instruction =
            builtin::list_builtin_facet_keys(FacetKind::Instruction)[0].to_string();
        let workflow = WorkflowDefinitionYaml {
            name: "builtin-facet".to_string(),
            description: "builtin facet diagnostic".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some(&builtin_instruction))],
        };
        save_workflow_yaml(tmp.path(), &workflow);

        // When
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        assert!(!report.items.iter().any(|item| {
            item.code == "FAC002" && item.facet_key.as_deref() == Some(&builtin_instruction)
        }));
        assert!(report.items.iter().any(|item| {
            item.code == "FAC000"
                && item.facet_kind.as_deref() == Some("instruction")
                && item.facet_key.as_deref() == Some(&builtin_instruction)
        }));
    }

    #[test]
    pub fn test_診断_指定directoryで不正permission宣言がwfs002になる() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("permission.yml"),
            permission_yaml("unknown"),
        )
        .unwrap();

        // When
        let report = diagnose_directory(tmp.path()).unwrap();
        let item = report
            .items
            .iter()
            .find(|item| item.code == "WFS002")
            .expect("WFS002 diagnostic");

        // Then
        assert_eq!(item.stage, DiagnosticStage::ParseShape);
        assert_eq!(item.field.as_deref(), Some("permission"));
        assert_eq!(item.span.as_ref().unwrap().start_line, 7);
    }

    #[test]
    pub fn collect_all_facet_keys_preserves_healthy_kinds_when_one_inventory_fails() {
        // Given
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        setup_facet(tmp.path(), "knowledge", "known", "known content");
        // When
        let result = collect_all_facet_keys(tmp.path());
        // Then
        assert!(result.is_err());
    }

    #[test]
    pub fn reference_diagnostics_ignore_unreferenced_broken_facet_inventory() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        setup_facet(tmp.path(), "knowledge", "known", "known content");
        let workflow = WorkflowDefinitionYaml {
            name: "knowledge-only".to_string(),
            description: "knowledge-only diagnostic".to_string(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        knowledge: vec!["known".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let diagnostics = diagnose_workflow_facet_references(&workflow, tmp.path()).unwrap();

        assert!(diagnostics.is_empty());
    }

    #[test]
    pub fn reference_diagnostics_propagate_referenced_broken_facet_inventory_error() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("policies"), "not a directory").unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "custom-policy".to_string(),
            description: "custom policy diagnostic".to_string(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        policy: Some("custom-policy".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let error = diagnose_workflow_facet_references(&workflow, tmp.path()).unwrap_err();

        assert!(matches!(error, facet::FacetError::Io(_)));
    }

    #[test]
    pub fn reference_diagnostics_short_circuit_broken_inventory_for_builtin_facet() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("knowledge"), "not a directory").unwrap();
        let workflow = WorkflowDefinitionYaml {
            name: "builtin-knowledge".to_string(),
            description: "builtin knowledge diagnostic".to_string(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        knowledge: vec!["releash-thread-cli".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let diagnostics = diagnose_workflow_facet_references(&workflow, tmp.path()).unwrap();

        assert!(diagnostics.is_empty());
    }

    fn fixture_dir(kind: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/adaptor/gateway/workflow/fixtures")
            .join(kind)
    }

    fn canonical_example_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../workflows/examples/full-cycle-development.yml")
    }

    #[test]
    pub fn canonical_example_has_zero_diagnostics() {
        let source = fs::read_to_string(canonical_example_path()).unwrap();
        let diagnosis = diagnose_workflow_source(&source, Some("full-cycle-development"));

        assert!(
            diagnosis.diagnostics.is_empty(),
            "canonical example produced diagnostics: {:?}",
            diagnosis.diagnostics
        );
        assert_eq!(
            diagnosis
                .workflow
                .expect("zero diagnostics must yield a workflow")
                .name,
            "full-cycle-development"
        );
    }

    fn expected_stage_for_code(code: &str) -> DiagnosticStage {
        match &code[..3] {
            "WFR" | "WFU" => DiagnosticStage::Resolve,
            "WFT" => DiagnosticStage::Typecheck,
            "WFC" => DiagnosticStage::ControlFlow,
            _ => DiagnosticStage::ParseShape,
        }
    }

    #[test]
    pub fn workflow_fixture_suite_valid_has_zero_diagnostics() {
        for entry in fs::read_dir(fixture_dir("valid")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yml") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap();
            let diagnosis =
                diagnose_workflow_source(&source, path.file_stem().and_then(|stem| stem.to_str()));
            assert!(
                diagnosis.diagnostics.is_empty(),
                "valid fixture {} produced diagnostics: {:?}",
                path.display(),
                diagnosis.diagnostics,
            );
        }
    }

    #[test]
    pub fn canonical_example_passes_the_real_loader() {
        let source = fs::read_to_string(canonical_example_path()).unwrap();
        let diagnosis = diagnose_workflow_source(&source, Some("full-cycle-development"));
        assert!(
            diagnosis.diagnostics.is_empty(),
            "canonical example produced diagnostics: {:?}",
            diagnosis.diagnostics
        );
        let definition = diagnosis
            .workflow
            .expect("zero diagnostics must yield a workflow");

        let tmp = TempDir::new().unwrap();
        let workflow_path = tmp.path().join("full-cycle-development.yml");
        fs::write(&workflow_path, source).unwrap();
        for node in &definition.nodes {
            let NodeKind::Session(session) = &node.kind else {
                continue;
            };
            if let Some(policy) = &session.facets.policy {
                setup_facet(tmp.path(), "policies", policy, "test policy");
            }
            for knowledge in &session.facets.knowledge {
                setup_facet(tmp.path(), "knowledge", knowledge, "test knowledge");
            }
            if let Some(instruction) = &session.facets.instruction {
                setup_facet(tmp.path(), "instructions", instruction, "test instruction");
            }
        }

        let workflow =
            crate::adaptor::gateway::workflow::storage::load_workflow(&workflow_path, tmp.path())
                .expect("canonical example must pass the real loader");
        assert_eq!(workflow.name, "full-cycle-development");
        assert_eq!(workflow.nodes.len(), definition.nodes.len());
    }

    #[test]
    pub fn workflow_fixture_suite_invalid_fixtures_have_expected_diagnostic_code() {
        for entry in fs::read_dir(fixture_dir("invalid")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("yml") {
                continue;
            }
            let filename = path.file_name().and_then(|name| name.to_str()).unwrap();
            let expected_code = filename.split('_').next().unwrap();
            let source = fs::read_to_string(&path).unwrap();
            let diagnosis = diagnose_workflow_source(&source, Some(filename));
            let matching = diagnosis
                .diagnostics
                .iter()
                .filter(|item| item.code == expected_code)
                .collect::<Vec<_>>();
            assert!(
                !matching.is_empty(),
                "invalid fixture {} did not produce expected code {expected_code}: {:?}",
                path.display(),
                diagnosis.diagnostics
            );
            assert!(
                matching
                    .iter()
                    .any(|item| item.stage == expected_stage_for_code(expected_code)),
                "fixture {} produced {expected_code} with wrong stage: {matching:?}",
                path.display()
            );
            assert!(
                matching.iter().any(|item| item.span.is_some()),
                "fixture {} expected {expected_code} to carry a span: {matching:?}",
                path.display()
            );

            let load_error = crate::adaptor::gateway::workflow::storage::load_workflow(
                &path,
                path.parent().expect("fixture path must have a parent"),
            )
            .expect_err("every invalid fixture must be rejected by the real loader");
            assert!(
                matches!(
                    load_error,
                    crate::adaptor::gateway::workflow::storage::StorageError::Diagnostics(ref items)
                        if items.iter().any(|item| item.code == expected_code)
                ),
                "loader rejection for {} did not preserve expected code {expected_code}: {load_error:?}",
                path.display()
            );
        }
    }

    #[test]
    pub fn invalid_source_diagnostics_use_file_stem_workflow_key() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        fs::write(
            wf_dir.join("file-stem.yml"),
            r#"
name: yaml-name
description: invalid workflow with mismatched name
nodes:
  main:
    command: printf implement
    session:
      provider: claude
      facets:
        instruction: implement
"#,
        )
        .unwrap();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        let summary = report
            .workflow_summaries
            .get("file-stem")
            .expect("invalid workflow summary must use file stem");
        let items = report
            .items
            .iter()
            .filter(|item| item.workflow_name.as_deref() == Some("file-stem"))
            .collect::<Vec<_>>();
        // Then
        assert_eq!(summary.error_count, items.len());
        assert!(
            !report
                .items
                .iter()
                .any(|item| item.workflow_name.as_deref() == Some("yaml-name")),
            "invalid source diagnostics must not be keyed by YAML name: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_all_validates_facet_templates_with_workflow_context() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(
            wf_dir,
            "instructions",
            "bad",
            "Use {{ missing_node }} and {{ item.path }}",
        );
        let wf = WorkflowDefinitionYaml {
            name: "semantic-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        for reference in ["missing_node", "item"] {
            assert!(
                report.items.iter().any(|item| item.code == "WFR003"
                    && item.workflow_name.as_deref() == Some("semantic-template")
                    && item.node_name.as_deref() == Some("main")
                    && item.facet_key.as_deref() == Some("bad")
                    && item.field.as_deref() == Some("content")
                    && item.message.contains(reference)
                    && item.span.is_some()),
                "expected semantic facet diagnostic for '{reference}', got: {:?}",
                report.items
            );
        }
        let workflow_summary = report
            .workflow_summaries
            .get("semantic-template")
            .expect("semantic facet errors must count toward workflow summary");
        // Then
        assert!(workflow_summary.error_count >= 2);
        let facet_summary = report
            .facet_summaries
            .get("instruction/bad")
            .expect("semantic facet errors must count toward facet summary");
        assert!(facet_summary.error_count >= 2);
    }

    #[test]
    pub fn invalid_rule_span_points_to_specific_rule_field() {
        let source =
            fs::read_to_string(fixture_dir("invalid").join("WFT001_when-on-enum.yml")).unwrap();
        let span_map = YamlSpanMap::parse(&source).unwrap();
        let expected = span_map
            .field_span("nodes.main.sequence.children[0].judge.rules[0].when.on")
            .expect("fixture must have when.on span");
        let diagnosis = diagnose_workflow_source(&source, Some("WFT001_when-on-enum"));
        let item = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFT001")
            .expect("fixture must produce WFT001");
        assert_eq!(item.field.as_deref(), Some("rules.when.on"));
        assert_eq!(item.span, Some(expected));
    }

    #[test]
    pub fn unreachable_subgraph_targets_are_not_marked_reachable() {
        let source =
            fs::read_to_string(fixture_dir("invalid").join("WFC001_unreachable-subgraph.yml"))
                .unwrap();
        let diagnosis = diagnose_workflow_source(&source, Some("unreachable-subgraph"));
        for node_name in ["orphan", "target"] {
            assert!(
                diagnosis
                    .diagnostics
                    .iter()
                    .any(|item| item.code == "WFC001"
                        && item.stage == DiagnosticStage::ControlFlow
                        && item.node_name.as_deref() == Some(node_name)),
                "expected WFC001 for {node_name}, got: {:?}",
                diagnosis.diagnostics
            );
        }
    }

    #[test]
    pub fn diagnose_broken_yaml() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path().join("workflows");
        fs::create_dir_all(&wf_dir).unwrap();
        fs::write(wf_dir.join("broken.yml"), "invalid: yaml: [[[").unwrap();

        // When
        let report = diagnose_all(&wf_dir, &wf_dir).unwrap();
        // Then
        assert!(
            report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error
                    && i.workflow_name.as_deref() == Some("broken"))
        );
    }

    #[test]
    pub fn diagnose_missing_facet_ref() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "knowledge", "known", "known content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        knowledge: vec!["known".to_string(), "missing-knowledge".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        let missing = report
            .items
            .iter()
            .find(|item| item.code == "FAC002")
            .expect("missing knowledge FAC002");
        // Then
        assert_eq!(missing.workflow_name.as_deref(), Some("test-wf"));
        assert_eq!(missing.node_name.as_deref(), Some("main"));
        assert_eq!(missing.facet_key.as_deref(), Some("missing-knowledge"));
        assert_eq!(missing.facet_kind.as_deref(), Some("knowledge"));
        assert_eq!(missing.field.as_deref(), Some("knowledge"));
        assert!(missing.message.contains("missing-knowledge"));
    }

    #[test]
    pub fn diagnose_missing_input_schema_ref() {
        // Given
        // Scenario: input が存在しない schemas Contract キーを参照していれば
        // workflow validation 経由でエラーになる
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                input: vec![InputParam {
                    name: "item".to_string(),
                    contract: Some("nonexistent-contract".to_string()),
                }],
                ..make_node("main", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.node_name.as_deref() == Some("main")
                && i.field.as_deref() == Some("input")),
            "Expected missing-input-schema error, got: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_missing_artifact_schema_ref_remains_node_scoped() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                artifact: Some("nonexistent-contract".to_string()),
                ..make_node("main", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.message.contains("nonexistent-contract")
                && i.node_name.as_deref() == Some("main")
                && i.field.as_deref() == Some("artifact")),
            "Expected missing-artifact-schema error on main, got: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_array_items_unknown_schema_ref_is_schema_scoped() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: [(
                "review-list".to_string(),
                SchemaDef::Array {
                    items: "missing-item".to_string(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![make_node("main", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("schemas.review-list")
                && i.message
                    .contains("array.items references unknown schemas 'missing-item'")
                && i.node_name.is_none()
                && i.field.as_deref() == Some("schemas")),
            "Expected schema-scoped array.items error, got: {:?}",
            report.items
        );
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.node_name.as_deref() == Some("review-list")),
            "array.items diagnostics must not be attached to a schema name as a node: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_schema_refs_do_not_record_facet_usage() {
        // Given
        // Scenario: schemas Contract はファセットではないため facet_usage に記録されない
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: [(
                "input-contract".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinition {
                input: vec![InputParam {
                    name: "item".to_string(),
                    contract: Some("input-contract".to_string()),
                }],
                ..make_node("main", Some("impl"))
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            !report.facet_usage.contains_key("contracts/input-contract"),
            "schemas Contract must not be tracked as facet usage: {:?}",
            report.facet_usage
        );
    }

    #[test]
    pub fn diagnose_missing_input_schema_ref_in_fanout_child() {
        // Given
        // Scenario: fanout child の input でも存在しない schemas Contract
        // 参照を検出する
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                make_fanout("main", vec!["child1"]),
                NodeDefinition {
                    input: vec![InputParam {
                        name: "item".to_string(),
                        contract: Some("nonexistent-contract".to_string()),
                    }],
                    ..make_child("child1", Some("impl"))
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.message.contains("存在しない schemas Contract")
                && i.node_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("input")),
            "Expected missing-input-schema error on child, got: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_missing_node_ref() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    kind: NodeKind::Sequence(
                        crate::adaptor::gateway::workflow::schema::SequenceSpec {
                            entry: None,
                            children: vec![crate::domain::workflow::ChildEntry {
                                name: "work".to_string(),
                                inputs: Vec::new(),
                                rules: Some(vec![Rule::Next("nonexistent".to_string())]),
                            }],
                        },
                    ),
                    ..make_node("main", None)
                },
                make_node("work", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        let rule_target_errors = report
            .items
            .iter()
            .filter(|i| {
                i.severity == Severity::Error
                    && i.node_name.as_deref() == Some("work")
                    && i.field.as_deref() == Some("rules.next")
                    && i.message.contains("存在しないnode")
            })
            .count();
        // Then
        assert_eq!(
            rule_target_errors, 1,
            "expected one rules target diagnostic from validate_all, got: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_unreachable_node() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // main sequence は start → node3 の隣接辺のみ → orphan は到達不能
        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                NodeDefinition {
                    kind: NodeKind::Sequence(
                        crate::adaptor::gateway::workflow::schema::SequenceSpec {
                            entry: None,
                            children: vec![
                                crate::domain::workflow::ChildEntry::reference("start"),
                                crate::domain::workflow::ChildEntry::reference("node3"),
                            ],
                        },
                    ),
                    ..make_node("main", None)
                },
                make_node("start", Some("impl")),
                make_node("orphan", Some("impl")),
                make_node("node3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            report.items.iter().any(|i| i.code == "WFC001"
                && i.severity == Severity::Error
                && i.stage == DiagnosticStage::ControlFlow
                && i.node_name.as_deref() == Some("orphan")),
            "Expected WFC001 for orphan, got: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_rules_without_fallthrough_marks_later_nodes_unreachable() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        // rules なしの node は終端なので、定義順の暗黙到達はない。
        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![
                make_node("main", Some("impl")),
                make_node("node2", Some("impl")),
                make_node("node3", Some("impl")),
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        for node_name in ["node2", "node3"] {
            assert!(
                report.items.iter().any(|i| i.code == "WFC001"
                    && i.severity == Severity::Error
                    && i.stage == DiagnosticStage::ControlFlow
                    && i.node_name.as_deref() == Some(node_name)),
                "Expected WFC001 for {node_name}, got: {:?}",
                report.items
            );
        }
    }

    #[test]
    pub fn diagnose_builtin_workflow_info() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Info && i.message.contains("ビルトインワークフロー")));
    }

    #[test]
    pub fn builtin_workflows_produce_no_diagnostic_errors() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        let errors: Vec<_> = report
            .items
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        // Then
        assert!(
            errors.is_empty(),
            "ビルトインワークフローに診断エラー: {errors:?}"
        );
    }

    #[test]
    pub fn diagnose_builtin_facet_info() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(report
            .items
            .iter()
            .any(|i| i.severity == Severity::Info && i.message.contains("ビルトインファセット")));
    }

    #[test]
    pub fn diagnose_template_variable_error() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "bad", "Use {{spec..b}} here");
        let wf = WorkflowDefinitionYaml {
            name: "bad-template".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("bad"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("bad")
            && i.message.contains("未定義のテンプレート変数 '{{spec..b}}'")));
    }

    #[test]
    pub fn diagnose_request_reference_ok() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "good", "Request: {{ request }}");

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(!report.items.iter().any(|i| i.severity == Severity::Error
            && i.facet_key.as_deref() == Some("good")
            && i.message.contains("未定義のテンプレート変数")));
    }

    /// command node は command を持ち facet は不要。
    /// diagnose_all 経路で valid な command node が誤って「ファセット参照が必要」
    /// エラーにならないことを担保する（validation.rs と同じ整合性が diagnostics 側にも必要）。
    #[test]
    pub fn diagnose_command_node_with_command_has_no_facet_required_error() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = WorkflowDefinitionYaml {
            name: "command-wf".to_string(),
            description: "command test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_command("main", "cargo build")],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.node_name.as_deref() == Some("main")
                && i.message.contains("ファセット参照")),
            "command node with command must not trigger facet requirement error: {:?}",
            report.items
        );
    }

    /// command node の command が空なら validation 経路で command field のエラーになる。
    #[test]
    pub fn diagnose_command_node_with_empty_command_reports_command_error() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();

        let wf = WorkflowDefinitionYaml {
            name: "command-wf".to_string(),
            description: "command test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_command("main", "   ")],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error && i.message.contains("command")),
            "command node without command must report a command-related error: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_facet_usage_tracked() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "impl", "content");

        let wf = WorkflowDefinitionYaml {
            name: "test-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("impl"))],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        let usage = report.facet_usage.get("instruction/impl");
        // Then
        assert!(usage.is_some());
        assert_eq!(usage.unwrap().len(), 1);
        assert_eq!(usage.unwrap()[0].workflow_name, "test-wf");
    }

    #[test]
    pub fn diagnose_tracks_each_knowledge_reference_usage() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "knowledge", "first", "first content");
        setup_facet(wf_dir, "knowledge", "second", "second content");

        let wf = WorkflowDefinitionYaml {
            name: "knowledge-usage".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        knowledge: vec!["first".to_string(), "second".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        for facet_id in ["knowledge/first", "knowledge/second"] {
            let usages = report
                .facet_usage
                .get(facet_id)
                .unwrap_or_else(|| panic!("missing usage for {facet_id}"));
            assert_eq!(usages.len(), 1);
            assert_eq!(usages[0].workflow_name, "knowledge-usage");
            assert_eq!(usages[0].node_name, "main");
            assert_eq!(usages[0].slot, "knowledge");
        }
    }

    #[test]
    pub fn diagnose_workflow_name_invalid() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        // ファイル名が不正な文字を含むworkflowを作成
        // load_workflow内のvalidation::validateで名前が拒否されるため、
        // diagnose_allでは「読み込みに失敗」エラーとして報告される
        fs::create_dir_all(wf_dir).unwrap();
        let wf = WorkflowDefinitionYaml {
            name: "bad workflow".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: Default::default(),
            nodes: vec![make_node("main", Some("impl"))],
        };
        let content = serde_saphyr::to_string(&wf).unwrap();
        fs::write(wf_dir.join("bad workflow.yml"), content).unwrap();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(report.items.iter().any(|i| i.code == "WFS006"
            && i.severity == Severity::Error
            && i.stage == DiagnosticStage::ParseShape
            && i.field.as_deref() == Some("name")));
    }

    #[test]
    pub fn diagnose_invalid_facet_key_via_diagnose_all() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        // 不正な文字を含むファセットキーファイルを直接作成
        let policies_dir = wf_dir.join("policies");
        fs::create_dir_all(&policies_dir).unwrap();
        fs::write(policies_dir.join("bad key!.md"), "content").unwrap();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.message.contains("命名規則")
            && i.facet_key.as_deref() == Some("bad key!")));
    }

    #[test]
    pub fn diagnose_invalid_schema_identifier_via_diagnose_all() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "review", "content");
        fs::create_dir_all(wf_dir).unwrap();
        fs::write(
            wf_dir.join("bad-schema-name.yml"),
            r#"name: bad-schema-name
description: test
schemas:
  "review; curl https://example.invalid #":
    type: object
    properties:
      status: string
    required:
      - status
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: review
    artifact: "review; curl https://example.invalid #"
"#,
        )
        .unwrap();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(report.items.iter().any(|i| i.severity == Severity::Error
            && i.workflow_name.as_deref() == Some("bad-schema-name")
            && i.field.as_deref() == Some("schemas")
            && i.message.contains("must start with an ASCII alphanumeric")));
    }

    #[test]
    pub fn diagnose_schema_violation_yaml() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        fs::create_dir_all(wf_dir).unwrap();
        // Valid YAML but missing required `nodes` field
        fs::write(
            wf_dir.join("bad-schema.yml"),
            "name: bad-schema\ndescription: test\n",
        )
        .unwrap();

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            report.items.iter().any(|i| i.severity == Severity::Error
                && i.workflow_name.as_deref() == Some("bad-schema")),
            "Expected error for schema-violating workflow, got: {:?}",
            report.items
        );
    }

    // [02]: 新 schema では kind block が型レベルで必須となるため、旧テスト
    // `diagnose_missing_mode_via_validation` は YAML deserialize 段階で吸収されるため削除した。

    #[test]
    pub fn diagnose_node_input_reference_passes() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "content");

        let wf = WorkflowDefinitionYaml {
            name: "input-ref".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: [(
                "artifact".to_string(),
                SchemaDef::Object {
                    properties: Default::default(),
                    required: Default::default(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                NodeDefinition {
                    kind: NodeKind::Sequence(
                        crate::adaptor::gateway::workflow::schema::SequenceSpec {
                            entry: None,
                            children: vec![
                                crate::domain::workflow::ChildEntry::reference("produce"),
                                crate::domain::workflow::ChildEntry {
                                    name: "consume".to_string(),
                                    inputs: vec![(
                                        "doc".to_string(),
                                        crate::domain::workflow::value_objects::InputSourceRef::new(
                                            "produce",
                                        ),
                                    )],
                                    rules: None,
                                },
                            ],
                        },
                    ),
                    ..make_node("main", None)
                },
                NodeDefinition {
                    artifact: Some("artifact".to_string()),
                    ..make_node("produce", Some("task"))
                },
                NodeDefinition {
                    input: vec![InputParam {
                        name: "doc".to_string(),
                        contract: None,
                    }],
                    artifact: Some("artifact".to_string()),
                    ..make_node("consume", Some("task"))
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            !report
                .items
                .iter()
                .any(|i| i.severity == Severity::Error && i.field.as_deref() == Some("inputs")),
            "children inputs wiring should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_fanout_child_item_reference_passes() {
        // Given
        let tmp = TempDir::new().unwrap();
        let wf_dir = tmp.path();
        setup_facet(wf_dir, "instructions", "task", "{{ item.path }}");

        let mut fanout = make_fanout("main", vec!["child1"]);
        let NodeKind::Fanout(fanout_spec) = &mut fanout.kind else {
            unreachable!();
        };
        fanout_spec.items = Some(ItemsSource::Literal(vec![serde_json::json!({
            "path": "src/lib.rs"
        })]));
        let wf = WorkflowDefinitionYaml {
            name: "par-item".to_string(),
            description: "test".to_string(),
            builtin: false,
            entry: "main".to_string(),
            schemas: [(
                "item-contract".to_string(),
                SchemaDef::Object {
                    properties: [("path".to_string(), SchemaDef::String { r#enum: None })]
                        .into_iter()
                        .collect(),
                    required: ["path".to_string()].into_iter().collect(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![
                fanout,
                NodeDefinition {
                    input: vec![InputParam {
                        name: "item".to_string(),
                        contract: Some("item-contract".to_string()),
                    }],
                    ..make_child("child1", Some("task"))
                },
            ],
        };
        save_workflow_yaml(wf_dir, &wf);

        // When
        let report = diagnose_all(wf_dir, wf_dir).unwrap();
        // Then
        assert!(
            !report.items.iter().any(|i| i.severity == Severity::Error
                && i.node_name.as_deref() == Some("child1")
                && i.field.as_deref() == Some("inputs")),
            "item reference inside fanout child should not be an error, got: {:?}",
            report.items
        );
    }

    #[test]
    pub fn diagnose_all_reports_lua_syntax_file_and_line() {
        // Given
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("broken.lua"),
            "local r = require('releash')\nreturn )",
        )
        .unwrap();

        // When
        let report = diagnose_all(tmp.path(), tmp.path()).unwrap();
        let diagnostic = report
            .items
            .iter()
            .find(|item| item.code == "WFS009")
            .expect("Lua syntax diagnostic");
        let span = diagnostic.span.as_ref().expect("Lua source span");

        // Then
        assert_eq!(span.source.as_deref(), Some("broken.lua"));
        assert_eq!(span.start_line, 2);
    }

    #[test]
    pub fn lua_and_yaml_domain_errors_keep_the_same_diagnostic_identity() {
        let tmp = TempDir::new().unwrap();
        let yaml = diagnose_workflow_source(
            r#"
name: review
description: Review
nodes:
  main:
    command: ""
"#,
            Some("review"),
        );
        let lua = diagnose_lua_workflow_source(
            "review.lua",
            r#"
local r = require("releash")
return r.workflow{
  name = "review", description = "Review",
  main = r.command{ command = "" },
}
"#,
            tmp.path(),
            tmp.path(),
            Some("review"),
        );
        let yaml_error = yaml
            .diagnostics
            .iter()
            .find(|item| item.severity == Severity::Error)
            .unwrap();
        let lua_error = lua
            .diagnostics
            .iter()
            .find(|item| item.severity == Severity::Error)
            .unwrap();

        assert_eq!(lua_error.code, yaml_error.code);
        assert_eq!(lua_error.stage, yaml_error.stage);
        assert_eq!(lua_error.message, yaml_error.message);
        assert_eq!(
            lua_error.span.as_ref().unwrap().source.as_deref(),
            Some("review.lua")
        );
    }

    #[test]
    pub fn test_command_env_yamlとluaが同じworkflow_definitionになる() {
        let tmp = TempDir::new().unwrap();
        let yaml = diagnose_workflow_source(
            r#"name: env-equivalence
description: env equivalence
schemas:
  document-contract:
    type: object
    properties:
      body:
        type: object
        properties:
          text: string
        required: [text]
    required:
      - body
nodes:
  main:
    command: printf
    input:
      - document: document-contract
      - context
    env:
      DOC: document
      BODY: document.body.text
      SPEC_DIR: context.spec_dir
"#,
            Some("env-equivalence"),
        );
        let lua = diagnose_lua_workflow_source(
            "env-equivalence.lua",
            r#"local r = require("releash")
local document_contract = r.schema.object{
  name = "document-contract",
  properties = { body = r.schema.object{
    properties = { text = r.schema.string{} }, required = { "text" },
  } },
  required = { "body" },
}
local document = r.input("document", document_contract)
local context = r.input("context")
return r.workflow{
  name = "env-equivalence", description = "env equivalence",
  main = r.command{
    command = "printf",
    input = { document, context },
    env = { DOC = document, BODY = document.body.text, SPEC_DIR = context.spec_dir },
  },
}
"#,
            tmp.path(),
            tmp.path(),
            Some("env-equivalence"),
        );

        assert!(!yaml.has_errors(), "{:?}", yaml.diagnostics);
        assert!(!lua.has_errors(), "{:?}", lua.diagnostics);
        assert_eq!(yaml.workflow, lua.workflow);
    }

    #[test]
    pub fn test_command_env_yamlとluaが同じ参照error_codeを返す() {
        let tmp = TempDir::new().unwrap();
        let cases = [
            (
                "env-not-map",
                r#"name: env-not-map
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env: document
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "env-not-map", description = "invalid env",
  main = r.command{ command = "true", input = { document }, env = document },
}
"#,
                "WFS002",
            ),
            (
                "env-value-not-input",
                r#"name: env-value-not-input
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      DOC: 2
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "env-value-not-input", description = "invalid env",
  main = r.command{ command = "true", input = { document }, env = { DOC = 2 } },
}
"#,
                "WFS002",
            ),
            (
                "unknown-input",
                r#"name: unknown-input
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      DOC: missing
"#,
                r#"local r = require("releash")
local document = r.input("document")
local missing = r.input("missing")
return r.workflow{
  name = "unknown-input", description = "invalid env",
  main = r.command{ command = "true", input = { document }, env = { DOC = missing } },
}
"#,
                "WFR003",
            ),
            (
                "unknown-contract-field",
                r#"name: unknown-contract-field
description: invalid env
schemas:
  document-contract:
    type: object
    properties:
      body: string
    required:
      - body
nodes:
  main:
    command: "true"
    input:
      - document: document-contract
    env:
      DOC: document.missing
"#,
                r#"local r = require("releash")
local document_contract = r.schema.object{
  name = "document-contract",
  properties = { body = r.schema.string{} },
  required = { "body" },
}
local document = r.input("document", document_contract)
return r.workflow{
  name = "unknown-contract-field", description = "invalid env",
  main = r.command{
    command = "true", input = { document }, env = { DOC = document.missing },
  },
}
"#,
                "WFR003",
            ),
            (
                "reserved-env",
                r#"name: reserved-env
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      RELEASH_WORKTREE_PATH: document
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "reserved-env", description = "invalid env",
  main = r.command{
    command = "true", input = { document },
    env = { RELEASH_WORKTREE_PATH = document },
  },
}
"#,
                "WFR004",
            ),
            (
                "invalid-env-name",
                r#"name: invalid-env-name
description: invalid env
nodes:
  main:
    command: "true"
    input:
      - document
    env:
      BAD-NAME: document
"#,
                r#"local r = require("releash")
local document = r.input("document")
return r.workflow{
  name = "invalid-env-name", description = "invalid env",
  main = r.command{
    command = "true", input = { document }, env = { ["BAD-NAME"] = document },
  },
}
"#,
                "WFS006",
            ),
        ];

        for (name, yaml_source, lua_source, expected_code) in cases {
            let yaml = diagnose_workflow_source(yaml_source, Some(name));
            let lua = diagnose_lua_workflow_source(
                &format!("{name}.lua"),
                lua_source,
                tmp.path(),
                tmp.path(),
                Some(name),
            );
            assert!(yaml.has_errors(), "{name}: {:?}", yaml.diagnostics);
            assert!(lua.has_errors(), "{name}: {:?}", lua.diagnostics);
            let expected_stage = expected_stage_for_code(expected_code);
            assert!(
                yaml.diagnostics
                    .iter()
                    .any(|item| item.code == expected_code && item.stage == expected_stage),
                "{name}: {:?}",
                yaml.diagnostics
            );
            assert!(
                lua.diagnostics
                    .iter()
                    .any(|item| item.code == expected_code && item.stage == expected_stage),
                "{name}: {:?}",
                lua.diagnostics
            );
        }
    }

    #[test]
    pub fn test_command_env_command以外のnodeとenvというnode名を拒否する() {
        let tmp = TempDir::new().unwrap();
        for (kind, body, lua_node) in [
            (
                "session",
                "session:\n      provider: claude\n    env: {}",
                "r.session{ provider = r.provider.claude, env = {} }",
            ),
            (
                "fanout",
                "fanout:\n      children: [worker]\n    env: {}",
                "r.fanout{ children = {}, env = {} }",
            ),
            (
                "sequence",
                "sequence:\n      children: [worker]\n    env: {}",
                "r.sequence{ children = {}, env = {} }",
            ),
        ] {
            let source = format!(
                "name: {kind}-env\ndescription: invalid env\nnodes:\n  main:\n    {body}\n  worker:\n    command: true\n"
            );
            let diagnosis = diagnose_workflow_source(&source, None);
            assert!(
                diagnosis
                    .diagnostics
                    .iter()
                    .any(|item| item.code == "WFS002" && item.field.as_deref() == Some("env")),
                "{kind}: {:?}",
                diagnosis.diagnostics
            );
            assert!(diagnosis.workflow.is_none());

            let lua = diagnose_lua_workflow_source(
                &format!("{kind}-env.lua"),
                &format!(
                    "local r = require(\"releash\")\nreturn r.workflow{{ name = \"{kind}-env\", description = \"invalid env\", main = {lua_node} }}"
                ),
                tmp.path(),
                tmp.path(),
                Some(&format!("{kind}-env")),
            );
            assert!(
                lua.diagnostics.iter().any(|item| item.code == "WFS002"),
                "{kind}: {:?}",
                lua.diagnostics
            );
            assert!(lua.workflow.is_none());
        }

        for reserved_name in ["env", "session"] {
            let workflow_name = format!("{reserved_name}-node-name");
            let node_name = diagnose_workflow_source(
                &format!(
                    "name: {workflow_name}\ndescription: reserved node name\nnodes:\n  main:\n    command: true\n  {reserved_name}:\n    command: true\n"
                ),
                None,
            );
            assert!(node_name.diagnostics.iter().any(|item| {
                item.code == "WFR004" && item.node_name.as_deref() == Some(reserved_name)
            }));
            assert!(node_name.workflow.is_none());

            let lua_node_name = diagnose_lua_workflow_source(
                &format!("{workflow_name}.lua"),
                &format!(
                    "local r = require(\"releash\")\nlocal child = r.command{{ name = \"{reserved_name}\", command = \"true\" }}\nreturn r.workflow{{\n  name = \"{workflow_name}\", description = \"reserved node name\",\n  main = r.sequence{{ children = {{ r.child{{ node = child }} }} }},\n}}\n"
                ),
                tmp.path(),
                tmp.path(),
                Some(&workflow_name),
            );
            assert!(lua_node_name
                .diagnostics
                .iter()
                .any(|item| item.code == "WFR004"));
            assert!(lua_node_name.workflow.is_none());
        }
    }

    #[test]
    pub fn test_workflow診断_module評価中にhostを呼ぶ定義と他定義を正常に診断する() {
        // Given
        let tmp = TempDir::new().unwrap();
        let module_dir = tmp.path().join("module-host-parts");
        fs::create_dir(&module_dir).unwrap();
        fs::write(
            module_dir.join("nodes.lua"),
            r#"local r = require("releash")
return { main = r.command{ command = "true" } }
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("module-host.lua"),
            r#"local r = require("releash")
local nodes = require("module-host-parts.nodes")
return r.workflow{
  name = "module-host", description = "Module host workflow", main = nodes.main,
}
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("healthy.lua"),
            r#"local r = require("releash")
return r.workflow{
  name = "healthy", description = "Healthy workflow",
  main = r.command{ command = "printf healthy" },
}
"#,
        )
        .unwrap();

        // When
        let loaded = load_workflows_in_scope(
            tmp.path(),
            tmp.path(),
            DiagnosticScope::ReachableFromDirectory,
        )
        .unwrap();
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        for name in ["module-host", "healthy"] {
            let (_, result) = loaded
                .iter()
                .find(|(workflow_name, _)| workflow_name == name)
                .unwrap_or_else(|| panic!("{name} の診断 load 結果"));
            let Ok((workflow, source_diagnostics)) = result else {
                panic!("{name} は診断時に正常 load されるべき: {result:?}");
            };
            assert_eq!(workflow.name, name);
            assert!(source_diagnostics.is_empty(), "{source_diagnostics:?}");
            assert!(!report.items.iter().any(|item| {
                item.workflow_name.as_deref() == Some(name) && item.severity == Severity::Error
            }));
        }

        assert!(!report.items.iter().any(|item| {
            item.workflow_name.as_deref() == Some("module-host")
                && matches!(item.code.as_str(), "WFS009" | "WFS010" | "WFS011")
        }));
    }

    #[test]
    pub fn test_workflow診断_lua定義のload失敗を位置付きで報告して他定義へ波及させない() {
        // Given
        let tmp = TempDir::new().unwrap();
        let module_dir = tmp.path().join("broken-parts");
        fs::create_dir(&module_dir).unwrap();
        fs::write(
            module_dir.join("nodes.lua"),
            "local r = require(\"releash\")\nreturn r.command{ command = }\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("broken.lua"),
            r#"local r = require("releash")
local nodes = require("broken-parts.nodes")
return r.workflow{
  name = "broken", description = "Broken workflow", main = nodes.main,
}
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("invalid-yaml.yml"),
            r#"name: invalid-yaml
description: Invalid YAML workflow
nodes:
  main:
    artifact: something
"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("healthy.lua"),
            r#"local r = require("releash")
return r.workflow{
  name = "healthy", description = "Healthy workflow",
  main = r.command{ command = "printf healthy" },
}
"#,
        )
        .unwrap();

        // When
        let loaded = load_workflows_in_scope(
            tmp.path(),
            tmp.path(),
            DiagnosticScope::ReachableFromDirectory,
        )
        .unwrap();
        let report = diagnose_directory(tmp.path()).unwrap();

        // Then
        let broken = report
            .items
            .iter()
            .find(|item| item.workflow_name.as_deref() == Some("broken") && item.code == "WFS009")
            .expect("broken module の syntax diagnostic");
        let span = broken.span.as_ref().expect("broken module の失敗位置");
        assert_eq!(span.source.as_deref(), Some("broken-parts/nodes.lua"));
        assert_eq!(span.start_line, 2);

        assert!(report.items.iter().any(|item| {
            item.workflow_name.as_deref() == Some("invalid-yaml")
                && item.severity == Severity::Error
        }));

        let (_, healthy_result) = loaded
            .iter()
            .find(|(name, _)| name == "healthy")
            .expect("healthy の診断 load 結果");
        let Ok((healthy, source_diagnostics)) = healthy_result else {
            panic!("healthy は正常 load されるべき: {healthy_result:?}");
        };
        assert_eq!(healthy.name, "healthy");
        assert!(source_diagnostics.is_empty(), "{source_diagnostics:?}");
        assert!(!report.items.iter().any(|item| {
            item.workflow_name.as_deref() == Some("healthy") && item.severity == Severity::Error
        }));
    }

    #[test]
    pub fn lua_component_error_uses_workflow_relative_source_and_line() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("component.lua"),
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

        let diagnosis = diagnose_lua_workflow_source(
            "review.lua",
            r#"
local component = require("component")
return require("releash").workflow{
  name = "review", description = "Review", main = component(),
}
"#,
            tmp.path(),
            tmp.path(),
            Some("review"),
        );
        let diagnostic = diagnosis
            .diagnostics
            .iter()
            .find(|item| item.code == "WFR003")
            .expect("component reference diagnostic");
        let span = diagnostic.span.as_ref().expect("component source span");

        assert_eq!(span.source.as_deref(), Some("component.lua"));
        assert_eq!(span.start_line, 5);
    }

    #[test]
    pub fn test_多段参照診断_yamlとluaでcode_stage_messageが一致する() {
        let tmp = TempDir::new().unwrap();
        let cases = [
            (
                "wiring",
                "WFR007",
                r#"name: wiring
description: wiring
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    sequence:
      children:
        - source
        - target:
            inputs:
              value: source.outer.missing
  source:
    command: source
    artifact: result
  target:
    command: target
    input: [value]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target", input = { r.input("value") } }
return r.workflow{ name = "wiring", description = "wiring", main = r.sequence{ children = {
  r.child{ node = source },
  r.child{ node = target, inputs = { value = source.outer.missing } },
} } }
"#,
            ),
            (
                "env",
                "WFR003",
                r#"name: env
description: env
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: target
    input:
      - doc: document
    env:
      VALUE: doc.outer.missing
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "env", description = "env", main = r.command{
  command = "target", input = { doc }, env = { VALUE = doc.outer.missing },
} }
"#,
            ),
            (
                "template",
                "WFR003",
                r#"name: template
description: template
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: "echo {{ doc.outer.missing }}"
    input:
      - doc: document
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "template", description = "template", main = r.command{
  command = "echo {{ doc.outer.missing }}", input = { doc },
} }
"#,
            ),
            (
                "when",
                "WFT001",
                r#"name: when
description: when
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          flag:
            type: boolean
        required: [flag]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - when:
                  on: outer.missing
                  then: "yes"
                next: "no"
        - "yes"
        - "no"
  source:
    command: source
    artifact: result
  "yes":
    command: yes
  "no":
    command: no
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { flag = r.schema.boolean() }, required = { "flag" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local yes = r.command{ name = "yes", command = "yes" }
local no = r.command{ name = "no", command = "no" }
return r.workflow{ name = "when", description = "when", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.when{ on = source.outer.missing, on_true = yes, next = no } } },
  r.child{ node = yes }, r.child{ node = no },
} } }
"#,
            ),
            (
                "switch",
                "WFT002",
                r#"name: switch
description: switch
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          verdict:
            type: string
            enum: [A]
        required: [verdict]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - switch:
                  on: outer.missing
                  cases:
                    A: target
        - target
  source:
    command: source
    artifact: result
  target:
    command: target
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { verdict = r.schema.string{ enum = { "A" } } }, required = { "verdict" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target" }
return r.workflow{ name = "switch", description = "switch", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.switch{ on = source.outer.missing, cases = { A = target } } } },
  r.child{ node = target },
} } }
"#,
            ),
            (
                "items",
                "WFR003",
                r#"name: items
description: items
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          values:
            type: array
            items: item
  item: string
nodes:
  main:
    sequence:
      children: [source, spread]
  source:
    command: source
    artifact: result
  spread:
    fanout:
      items: source.outer.missing
      children: [worker]
  worker:
    command: worker
    input: [item]
"#,
                r#"local r = require("releash")
local item = r.schema.string{}
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { values = r.schema.array{ items = item } } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local worker = r.command{ name = "worker", command = "worker", input = { r.input("item") } }
local spread = r.fanout{ name = "spread", items = source.outer.missing, children = { r.child{ node = worker } } }
return r.workflow{ name = "items", description = "items", main = r.sequence{ children = {
  r.child{ node = source }, r.child{ node = spread },
} } }
"#,
            ),
            (
                "wiring-intermediate-missing",
                "WFR007",
                r#"name: wiring-intermediate-missing
description: wiring intermediate missing
schemas:
  result:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    sequence:
      children:
        - source
        - target:
            inputs:
              value: source.outer.missing.value
  source:
    command: source
    artifact: result
  target:
    command: target
    input: [value]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target", input = { r.input("value") } }
return r.workflow{ name = "wiring-intermediate-missing", description = "wiring intermediate missing", main = r.sequence{ children = {
  r.child{ node = source },
  r.child{ node = target, inputs = { value = source.outer.missing.value } },
} } }
"#,
            ),
            (
                "when-intermediate-missing",
                "WFT001",
                r#"name: when-intermediate-missing
description: when intermediate missing
schemas:
  result:
    type: object
    properties:
      route:
        type: object
        properties:
          flag:
            type: boolean
        required: [flag]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - when:
                  on: route.missing.flag
                  then: "yes"
                next: "no"
        - "yes"
        - "no"
  source:
    command: source
    artifact: result
  "yes":
    command: yes
  "no":
    command: no
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  route = r.schema.object{ properties = { flag = r.schema.boolean() }, required = { "flag" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local yes = r.command{ name = "yes", command = "yes" }
local no = r.command{ name = "no", command = "no" }
return r.workflow{ name = "when-intermediate-missing", description = "when intermediate missing", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.when{ on = source.route.missing.flag, on_true = yes, next = no } } },
  r.child{ node = yes }, r.child{ node = no },
} } }
"#,
            ),
            (
                "switch-intermediate-missing",
                "WFT002",
                r#"name: switch-intermediate-missing
description: switch intermediate missing
schemas:
  result:
    type: object
    properties:
      route:
        type: object
        properties:
          status:
            type: string
            enum: [ready]
        required: [status]
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - switch:
                  on: route.missing.status
                  cases:
                    ready: target
        - target
  source:
    command: source
    artifact: result
  target:
    command: target
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = {
  route = r.schema.object{ properties = { status = r.schema.string{ enum = { "ready" } } }, required = { "status" } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target" }
return r.workflow{ name = "switch-intermediate-missing", description = "switch intermediate missing", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.switch{ on = source.route.missing.status, cases = { ready = target } } } },
  r.child{ node = target },
} } }
"#,
            ),
            (
                "env-intermediate-missing",
                "WFR003",
                r#"name: env-intermediate-missing
description: env intermediate missing
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: target
    input:
      - doc: document
    env:
      VALUE: doc.outer.missing.value
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "env-intermediate-missing", description = "env intermediate missing", main = r.command{
  command = "target", input = { doc }, env = { VALUE = doc.outer.missing.value },
} }
"#,
            ),
            (
                "template-intermediate-missing",
                "WFR003",
                r#"name: template-intermediate-missing
description: template intermediate missing
schemas:
  document:
    type: object
    properties:
      outer:
        type: object
        properties:
          value: string
nodes:
  main:
    command: "echo {{ doc.outer.missing.value }}"
    input:
      - doc: document
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.object{ properties = { value = r.schema.string{} } },
} }
local doc = r.input("doc", document)
return r.workflow{ name = "template-intermediate-missing", description = "template intermediate missing", main = r.command{
  command = "echo {{ doc.outer.missing.value }}", input = { doc },
} }
"#,
            ),
            (
                "items-intermediate-missing",
                "WFR003",
                r#"name: items-intermediate-missing
description: items intermediate missing
schemas:
  result:
    type: object
    properties:
      batch:
        type: object
        properties:
          values:
            type: array
            items: item
  item: string
nodes:
  main:
    sequence:
      children: [source, spread]
  source:
    command: source
    artifact: result
  spread:
    fanout:
      items: source.batch.missing.values
      children: [worker]
  worker:
    command: worker
    input: [item]
"#,
                r#"local r = require("releash")
local item = r.schema.string{}
local result = r.schema.object{ properties = {
  batch = r.schema.object{ properties = { values = r.schema.array{ items = item } } },
} }
local source = r.command{ name = "source", command = "source", artifact = result }
local worker = r.command{ name = "worker", command = "worker", input = { r.input("item") } }
local spread = r.fanout{ name = "spread", items = source.batch.missing.values, children = { r.child{ node = worker } } }
return r.workflow{ name = "items-intermediate-missing", description = "items intermediate missing", main = r.sequence{ children = {
  r.child{ node = source }, r.child{ node = spread },
} } }
"#,
            ),
            (
                "wiring-non-object",
                "WFR007",
                r#"name: wiring-non-object
description: wiring non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children:
        - source
        - target:
            inputs:
              value: source.outer.value
  source:
    command: source
    artifact: result
  target:
    command: target
    input: [value]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target", input = { r.input("value") } }
return r.workflow{ name = "wiring-non-object", description = "wiring non-object", main = r.sequence{ children = {
  r.child{ node = source },
  r.child{ node = target, inputs = { value = source.outer.value } },
} } }
"#,
            ),
            (
                "env-non-object",
                "WFR003",
                r#"name: env-non-object
description: env non-object
schemas:
  document:
    type: object
    properties:
      outer: string
nodes:
  main:
    command: target
    input:
      - doc: document
    env:
      VALUE: doc.outer.value
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.string{},
} }
local doc = r.input("doc", document)
return r.workflow{ name = "env-non-object", description = "env non-object", main = r.command{
  command = "target", input = { doc }, env = { VALUE = doc.outer.value },
} }
"#,
            ),
            (
                "template-non-object",
                "WFR003",
                r#"name: template-non-object
description: template non-object
schemas:
  document:
    type: object
    properties:
      outer: string
nodes:
  main:
    command: "echo {{ doc.outer.value }}"
    input:
      - doc: document
"#,
                r#"local r = require("releash")
local document = r.schema.object{ name = "document", properties = {
  outer = r.schema.string{},
} }
local doc = r.input("doc", document)
return r.workflow{ name = "template-non-object", description = "template non-object", main = r.command{
  command = "echo {{ doc.outer.value }}", input = { doc },
} }
"#,
            ),
            (
                "when-non-object",
                "WFT001",
                r#"name: when-non-object
description: when non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - when:
                  on: outer.flag
                  then: "yes"
                next: "no"
        - "yes"
        - "no"
  source:
    command: source
    artifact: result
  "yes":
    command: yes
  "no":
    command: no
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local yes = r.command{ name = "yes", command = "yes" }
local no = r.command{ name = "no", command = "no" }
return r.workflow{ name = "when-non-object", description = "when non-object", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.when{ on = source.outer.flag, on_true = yes, next = no } } },
  r.child{ node = yes }, r.child{ node = no },
} } }
"#,
            ),
            (
                "switch-non-object",
                "WFT002",
                r#"name: switch-non-object
description: switch non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children:
        - source:
            rules:
              - switch:
                  on: outer.verdict
                  cases:
                    A: target
        - target
  source:
    command: source
    artifact: result
  target:
    command: target
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local target = r.command{ name = "target", command = "target" }
return r.workflow{ name = "switch-non-object", description = "switch non-object", main = r.sequence{ children = {
  r.child{ node = source, rules = { r.switch{ on = source.outer.verdict, cases = { A = target } } } },
  r.child{ node = target },
} } }
"#,
            ),
            (
                "items-non-object",
                "WFR003",
                r#"name: items-non-object
description: items non-object
schemas:
  result:
    type: object
    properties:
      outer: string
nodes:
  main:
    sequence:
      children: [source, spread]
  source:
    command: source
    artifact: result
  spread:
    fanout:
      items: source.outer.values
      children: [worker]
  worker:
    command: worker
    input: [item]
"#,
                r#"local r = require("releash")
local result = r.schema.object{ properties = { outer = r.schema.string{} } }
local source = r.command{ name = "source", command = "source", artifact = result }
local worker = r.command{ name = "worker", command = "worker", input = { r.input("item") } }
local spread = r.fanout{ name = "spread", items = source.outer.values, children = { r.child{ node = worker } } }
return r.workflow{ name = "items-non-object", description = "items non-object", main = r.sequence{ children = {
  r.child{ node = source }, r.child{ node = spread },
} } }
"#,
            ),
        ];

        for (name, code, yaml_source, lua_source) in cases {
            let yaml = diagnose_workflow_source(yaml_source, Some(name));
            let lua = diagnose_lua_workflow_source(
                &format!("{name}.lua"),
                lua_source,
                tmp.path(),
                tmp.path(),
                Some(name),
            );
            let yaml_item = yaml
                .diagnostics
                .iter()
                .find(|item| item.code == code)
                .unwrap_or_else(|| panic!("{name} YAML: {:?}", yaml.diagnostics));
            let lua_item = lua
                .diagnostics
                .iter()
                .find(|item| item.code == code)
                .unwrap_or_else(|| panic!("{name} Lua: {:?}", lua.diagnostics));
            assert_eq!(lua_item.code, yaml_item.code, "{name}");
            assert_eq!(lua_item.stage, yaml_item.stage, "{name}");
            assert_eq!(lua_item.message, yaml_item.message, "{name}");
        }
    }
}
