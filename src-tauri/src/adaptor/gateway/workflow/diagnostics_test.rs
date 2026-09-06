use super::*;

const MERGED_REFERENCES: &str = include_str!("fixtures/valid/sequence-merged-references.yml");

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
