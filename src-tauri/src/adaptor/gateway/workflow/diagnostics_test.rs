use super::*;

const MERGED_REFERENCES: &str = include_str!("fixtures/valid/sequence-merged-references.yml");

#[test]
fn test_lua未消費参照の診断_非object契約のleafを唯一の診断で指す() {
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
fn test_sequence宣言の診断_luaの入れ子とrequire先のartifact位置を指す() {
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
fn test_sequence宣言の診断_yamlとluaでoutputとartifactを同じcodeとstageで拒否する() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let cases = [
        (
            "output",
            "WFS002",
            include_str!("fixtures/invalid/WFS002_sequence-output.yml"),
        ),
        (
            "artifact",
            "WFS008",
            include_str!("fixtures/invalid/WFS008_sequence-artifact.yml"),
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
fn test_sequence多段参照の診断_配線と述語の未解決と末端型を既存codeで拒否する() {
    // Given
    let cases = [
        (
            "flag: review_scan.check_full_review_threads.has_open_threads",
            "flag: review_scan.check_full_review_threads.missing",
            "WFR007",
            DiagnosticStage::Resolve,
        ),
        (
            "on: check_full_review_threads.has_open_threads",
            "on: check_full_review_threads.missing",
            "WFT001",
            DiagnosticStage::Typecheck,
        ),
        (
            "on: check_full_review_threads.has_open_threads",
            "on: check_full_review_threads.status",
            "WFT001",
            DiagnosticStage::Typecheck,
        ),
        (
            "on: classify.status",
            "on: classify.missing",
            "WFT002",
            DiagnosticStage::Typecheck,
        ),
        (
            "on: classify.status",
            "on: classify.has_open_threads",
            "WFT002",
            DiagnosticStage::Typecheck,
        ),
        (
            "required: [has_open_threads, status, tasks]",
            "required: [tasks]",
            "WFT001",
            DiagnosticStage::Typecheck,
        ),
        (
            "required: [has_open_threads, status, tasks]",
            "required: [has_open_threads, tasks]",
            "WFT002",
            DiagnosticStage::Typecheck,
        ),
    ];
    for (from, to, code, stage) in cases {
        // When
        let diagnosis = diagnose_workflow_source(&MERGED_REFERENCES.replace(from, to), None);

        // Then
        assert!(
            diagnosis.diagnostics.iter().any(|item| item.code == code
                && item.stage == stage
                && item.severity == Severity::Error),
            "{to}: {:?}",
            diagnosis.diagnostics
        );
    }
}

#[test]
fn test_sequence多段参照の診断_実loaderが統合mapの参照を受理する() {
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

const FANOUT_REFERENCES: &str = include_str!("fixtures/valid/fanout-map-references.yml");
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
fn test_fanout多段参照の診断_yamlとluaの配線とitemsを診断ゼロでloadする() {
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

#[test]
fn test_fanout多段参照の診断_未解決の配線をWFR007と絶対位置で報告する() {
    // Given
    for (from, to, message) in [
        (
            "named: fan.a.passed",
            "named: fan.missing.passed",
            "source node 'fan' Artifact does not declare segment 1 ('missing')",
        ),
        (
            "indexed: indexed.0.passed",
            "indexed: indexed.007.passed",
            "source node 'indexed' Artifact does not declare segment 1 ('007')",
        ),
        (
            "nested: seq.nested_fan.nested_a.passed",
            "nested: seq.nested_fan.nested_a.passed.value",
            "source node 'seq' Artifact cannot resolve segment 4 ('value') from a non-object value",
        ),
    ] {
        // When
        let diagnosis = diagnose_workflow_source(&FANOUT_REFERENCES.replace(from, to), None);

        // Then
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|item| item.code == "WFR007"
                    && item.stage == DiagnosticStage::Resolve
                    && item.message.contains(message)),
            "{to}: {:?}",
            diagnosis.diagnostics
        );
    }
}

const FANOUT_ROUTING: &str = include_str!("fixtures/valid/fanout-map-routing.yml");

#[test]
fn test_fanoutの判別規則の診断_終端の型とrequiredでWFT001とWFT002を返す() {
    // Given
    for (from, to, code) in [
        ("on: a.passed", "on: a.verdict", "WFT001"),
        ("on: a.passed", "on: a.missing", "WFT001"),
        ("on: classify.verdict", "on: classify.passed", "WFT002"),
        ("on: classify.verdict", "on: classify.missing", "WFT002"),
        (
            "required: [passed, verdict]",
            "required: [verdict]",
            "WFT001",
        ),
        (
            "required: [passed, verdict]",
            "required: [passed]",
            "WFT002",
        ),
        ("enum: [READY, HOLD]", "enum: []", "WFT002"),
        ("on: nested_fan.nested_a.passed", "on: nested_fan", "WFT001"),
    ] {
        // When
        let diagnosis = diagnose_workflow_source(&FANOUT_ROUTING.replace(from, to), None);

        // Then
        assert!(
            diagnosis.diagnostics.iter().any(|item| item.code == code
                && item.stage == DiagnosticStage::Typecheck
                && item.severity == Severity::Error),
            "{to}: {:?}",
            diagnosis.diagnostics
        );
        assert!(!diagnosis
            .diagnostics
            .iter()
            .any(|item| item.code == "WFT006"));
    }
}

#[test]
fn test_fanoutの判別規則の診断_yamlとluaのwhenとswitchと入れ子を診断ゼロでloadする() {
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
fn test_fanout変更後のbuiltin定義_8本すべて診断ゼロでloadする() {
    // Given
    let summaries = builtin::list_builtin_workflows();
    assert_eq!(summaries.len(), 8);
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

const PREDICATE_ROUTING: &str = include_str!("fixtures/valid/predicate-routing.yml");
const NESTED_PREDICATE: &str = "{and: [passed, {or: [clean, skipped]}]}";

fn predicate_yaml(on: &str) -> String {
    PREDICATE_ROUTING.replace(NESTED_PREDICATE, on)
}

#[test]
fn test_述語load_単一参照と合成とネストを診断ゼロで受理する() {
    // Given
    for on in [
        "passed",
        "ok",
        "legacy flag",
        "details.passed",
        "{and: [passed]}",
        "{or: [passed]}",
        "{and: [passed, clean]}",
        "{or: [passed, skipped]}",
        NESTED_PREDICATE,
    ] {
        // When
        let diagnosis = diagnose_workflow_source(&predicate_yaml(on), None);
        // Then
        assert!(
            diagnosis.workflow.is_some(),
            "{on}: {:?}",
            diagnosis.diagnostics
        );
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{on}: {:?}",
            diagnosis.diagnostics
        );
    }
}

#[test]
fn test_述語load_空と不正な構造はshapeで拒否する() {
    // Given
    for (on, message, end_col) in [
        (
            "{and: []}",
            "predicate and/or must contain at least one element",
            24,
        ),
        (
            "{or: []}",
            "predicate and/or must contain at least one element",
            24,
        ),
        (
            "{and: [passed, {or: []}]}",
            "predicate and/or must contain at least one element",
            24,
        ),
        (
            "{or: [passed, {and: []}]}",
            "predicate and/or must contain at least one element",
            24,
        ),
        (
            "{and: passed}",
            "predicate and/or must contain an array",
            24,
        ),
        ("{or: true}", "predicate and/or must contain an array", 24),
        ("{and: null}", "predicate and/or must contain an array", 24),
        (
            "{or: [passed, {and: false}]}",
            "predicate and/or must contain an array",
            24,
        ),
        (
            "{and: [passed], or: [clean]}",
            "predicate map must contain exactly one key: and or or",
            24,
        ),
        (
            "{or: [passed], extra: true}",
            "predicate map must contain exactly one key: and or or",
            24,
        ),
        (
            "{not: passed}",
            "predicate map must contain exactly one key: and or or",
            24,
        ),
        (
            "{}",
            "predicate map must contain exactly one key: and or or",
            24,
        ),
        (
            "{and: [passed, {not: clean}]}",
            "predicate map must contain exactly one key: and or or",
            24,
        ),
        (
            "{or: [passed, {and: [clean], or: [skipped]}]}",
            "predicate map must contain exactly one key: and or or",
            24,
        ),
        (
            "42",
            "predicate must be a field reference or an and/or map",
            25,
        ),
        (
            "true",
            "predicate must be a field reference or an and/or map",
            27,
        ),
        (
            "null",
            "predicate must be a field reference or an and/or map",
            27,
        ),
        (
            "[passed]",
            "predicate must be a field reference or an and/or map",
            24,
        ),
        (
            "{and: [passed, false]}",
            "predicate must be a field reference or an and/or map",
            24,
        ),
        (
            "{or: [passed, null]}",
            "predicate must be a field reference or an and/or map",
            24,
        ),
    ] {
        // When
        let diagnosis = diagnose_workflow_source(&predicate_yaml(on), None);
        // Then
        assert!(diagnosis.workflow.is_none(), "{on}");
        assert_eq!(
            diagnosis.diagnostics.len(),
            1,
            "{on}: {:?}",
            diagnosis.diagnostics
        );
        let diagnostic = &diagnosis.diagnostics[0];
        assert_eq!(diagnostic.code, "WFS002", "{on}");
        assert_eq!(diagnostic.stage, DiagnosticStage::ParseShape);
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.message, message, "{on}");
        assert_eq!(diagnostic.field.as_deref(), Some("rules.when.on"));
        assert_eq!(
            diagnostic.span,
            Some(DiagnosticSpan {
                source: None,
                start_line: 26,
                start_col: 23,
                end_line: 26,
                end_col,
            }),
            "{on}"
        );
    }
}

#[test]
fn test_述語load_全参照の型検査は単一参照の理由を保持する() {
    // Given
    for (field, reason) in [
        (
            "details.text",
            "routing field 'details.text' must be boolean or string enum",
        ),
        (
            "details.optional",
            "routing field 'details.optional' must be required on its parent Object",
        ),
        (
            "details.unknown",
            "routing field 'details.unknown' has undeclared segment 2 ('unknown')",
        ),
        (
            "passed.flag",
            "routing field 'passed.flag' cannot resolve segment 2 ('flag') from a non-object value",
        ),
        (
            "details..passed",
            "routing field 'details..passed' is not a valid field path",
        ),
        (
            " passed",
            "routing field ' passed' is not a valid field path",
        ),
        (
            "passed ",
            "routing field 'passed ' is not a valid field path",
        ),
    ] {
        let single = diagnose_workflow_source(&predicate_yaml(&format!("'{field}'")), None);
        let compound = diagnose_workflow_source(
            &predicate_yaml(&format!(
                "{{or: [passed, {{and: ['{field}', '{field}']}}]}}"
            )),
            None,
        );
        // When / Then
        assert!(single.has_errors());
        assert!(compound.has_errors());
        assert_eq!(
            single.diagnostics.len(),
            1,
            "{field}: {:?}",
            single.diagnostics
        );
        assert_eq!(
            compound.diagnostics.len(),
            2,
            "{field}: {:?}",
            compound.diagnostics
        );
        for diagnostic in single.diagnostics.iter().chain(&compound.diagnostics) {
            assert_eq!(diagnostic.code, "WFT001");
            assert_eq!(diagnostic.stage, DiagnosticStage::Typecheck);
            assert_eq!(diagnostic.severity, Severity::Error);
            assert_eq!(
                diagnostic.message,
                format!("node 'judge' のrulesが不正です: {reason}")
            );
        }
    }
}

const PREDICATE_LUA: &str = include_str!("fixtures/valid/predicate-routing.lua");

#[test]
fn test_lua容量の診断_述語再利用の上限超過は位置を保持してloadを拒否する() {
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
    let result = super::super::storage::load_workflow(&path, directory.path());
    // Then
    let Err(super::super::storage::StorageError::Diagnostics(diagnostics)) = result else {
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
fn test_lua参照解決の診断_変換不能な値は利用箇所ごとの理由とfieldでloadを拒否する() {
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
        let result = super::super::storage::load_workflow(&path, directory.path());
        // Then
        let Err(super::super::storage::StorageError::Diagnostics(diagnostics)) = result else {
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
fn test_lua述語の診断_自childのartifact_field以外はresolveでloadを拒否する() {
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
            let result = super::super::storage::load_workflow(&path, directory.path());
            // Then
            let Err(super::super::storage::StorageError::Diagnostics(diagnostics)) = result else {
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
fn test_述語の表面間同値性_受理と全真理値の遷移が一致する() {
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
fn test_述語の表面間同値性_空と型とrequiredとpathの診断が一致する() {
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
fn test_述語の表面間同値性_空と不正な要素と配列以外は同じ診断でloadを拒否する() {
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
            let result = super::super::storage::load_workflow(&path, directory.path());
            // Then
            let Err(super::super::storage::StorageError::Diagnostics(diagnostics)) = result else {
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
fn test_述語の表面間同値性_sequenceとfanoutの異なるslotを合成する() {
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
            let workflow = super::super::storage::load_workflow(&path, directory.path()).unwrap();
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
fn test_述語の実loader_不正なshapeと参照を両表面でloadしない() {
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
            let result = super::super::storage::load_workflow(&path, directory.path());
            // Then
            assert!(
                matches!(
                    result,
                    Err(super::super::storage::StorageError::Diagnostics(_))
                ),
                "{result:?}"
            );
        }
    }
}

#[test]
fn test_述語の回帰_builtinと正本サンプルの全18辺は単一参照の遷移を保つ() {
    use crate::domain::workflow::{
        services::routing::{route_in_scope, RouteDecision},
        Predicate,
    };
    // Given
    let mut sources: Vec<_> = builtin::list_builtin_workflows()
        .iter()
        .map(|summary| builtin::builtin_workflow_source(&summary.name).unwrap())
        .collect();
    sources.push(include_str!(
        "../../../../../workflows/examples/full-cycle-development.yml"
    ));
    let mut count = 0;
    for source in sources {
        let diagnosis = diagnose_workflow_source(source, None);
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{:?}",
            diagnosis.diagnostics
        );
        let workflow = diagnosis.workflow.unwrap();
        for sequence in workflow.nodes.iter().filter_map(|node| node.sequence()) {
            for child in &sequence.children {
                for rule in child.rules.iter().flatten() {
                    let Rule::When { on, then, next } = rule else {
                        continue;
                    };
                    let Predicate::Ref(field) = on else {
                        panic!("existing when must remain a single reference")
                    };
                    count += 1;
                    for (value, target) in [(true, then), (false, next)] {
                        let artifact = field.split('.').rev().fold(
                            serde_json::json!(value),
                            |value, segment| serde_json::json!({segment: value}),
                        );
                        // When
                        let decision = route_in_scope(
                            &workflow,
                            sequence,
                            &child.name,
                            Some(&artifact),
                            &HashMap::new(),
                        )
                        .unwrap();
                        // Then
                        assert_eq!(
                            decision,
                            RouteDecision::TransitionTo(target.clone()),
                            "{}: {}",
                            workflow.name,
                            child.name
                        );
                    }
                }
            }
        }
    }
    assert_eq!(count, 18);
}

#[test]
fn test_completion診断_yamlとluaの同じ誤りはcode_stageと各表面のmessageを保つ() {
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
fn test_completion診断_全node種別でyamlとluaが同じ要求の有無を持つ定義を構築する() {
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
fn test_completion移行_builtin8本と正本サンプルが診断なしで既存の承認要求を保持する() {
    // Given
    for (name, expected) in [
        ("01_author-spec", vec!["final_review"]),
        (
            "02_implement-existing-spec",
            vec!["implementation_confirmation"],
        ),
        ("03_full-review", vec![]),
        ("04_review-fix-policy", vec![]),
        (
            "04_review-fix-policy-manual",
            vec!["decide_fix_policies_manual", "policy_confirmation"],
        ),
        ("05_review-fix", vec![]),
        ("06_handle-pr-review", vec!["pr_review_confirmation"]),
        (
            "06_handle-pr-review-manual",
            vec![
                "decide_pr_review_fix_policies_manual",
                "pr_review_confirmation",
            ],
        ),
        (
            "full-cycle-development",
            vec!["implementation_confirmation", "spec_confirmation"],
        ),
    ] {
        let source = if name == "full-cycle-development" {
            include_str!("../../../../../workflows/examples/full-cycle-development.yml")
        } else {
            builtin::builtin_workflow_source(name).unwrap()
        };
        // When
        let diagnosis = diagnose_workflow_source(source, Some(name));
        // Then
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{name}: {:?}",
            diagnosis.diagnostics
        );
        let workflow = diagnosis.workflow.unwrap();
        let mut required = workflow
            .nodes
            .iter()
            .filter(|node| node.requires_approval_completion())
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>();
        required.sort_unstable();
        assert_eq!(required, expected, "{name}");
    }
}

#[test]
fn test_隔離定義_yamlとluaの全node種別でmodeを受理する() {
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
fn test_隔離定義_値域外のyaml値とluaの文字列や他のhandleを拒否する() {
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
fn test_隔離定義_宣言の有無を問わずcontract直下のworktreeを拒否する() {
    // Given
    for mode in ["", "worktree: shared,", "worktree: isolated,"] {
        for kind in [
            "command: 'true'",
            "session: {provider: codex, facets: {instruction: test}}",
        ] {
            let source = format!("name: reserved\ndescription: test\nschemas:\n  result: {{type: object, properties: {{worktree: string}}}}\nnodes:\n  main: {{{mode} {kind}, artifact: result}}");

            // When
            let diagnosis = diagnose_workflow_source(&source, None);

            // Then
            assert!(diagnosis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error));
            assert!(crate::domain::workflow::services::validation::validate(
                &diagnosis.workflow.unwrap()
            )
            .is_err());
            assert!(
                diagnosis
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("worktree")),
                "{:?}",
                diagnosis.diagnostics
            );
        }
    }
}

#[test]
fn test_隔離定義_合成子とcontractなしsessionを経由してworktreeを参照する() {
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
