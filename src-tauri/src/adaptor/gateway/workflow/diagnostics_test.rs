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
