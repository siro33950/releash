use super::*;
use serde_json::{json, Value};

fn definition() -> Value {
    json!({
        "name": "delegate", "description": "delegate tests",
        "schemas": {
            "parent-result": {"type": "object", "properties": {"done": {"type": "boolean"}, "task": "string"}, "required": ["done", "task"]},
            "verdict": {"type": "object", "properties": {"passed": {"type": "boolean"}, "skipped": {"type": "boolean"}}, "required": ["passed", "skipped"]},
            "text": {"type": "string"}, "flag": {"type": "boolean"}
        },
        "nodes": {
            "main": {"session": {"provider": "codex", "facets": {"instruction": "implement"}}, "artifact": "parent-result",
                "completion": {"delegate": {"child": "verify", "when": "child.passed", "max_iterations": 2}}},
            "verify": {"command": "check", "artifact": "verdict"}
        }
    })
}

fn check(value: &Value) -> WorkflowSourceDiagnostics {
    diagnose_workflow_source(&value.to_string(), None)
}

#[test]
fn test_delegate定義_承認との併記と述語の合成を受理して保存復元する() {
    // Given
    for approval in [false, true] {
        let mut value = definition();
        if approval {
            value["nodes"]["main"]["completion"]["require"] = json!("approval");
        }
        value["nodes"]["main"]["completion"]["delegate"]["when"] =
            json!({"and": ["done", {"or": ["child.passed", "child.skipped"]}]});
        // When
        let diagnosis = check(&value);
        // Then
        assert!(
            diagnosis.diagnostics.is_empty(),
            "{:?}",
            diagnosis.diagnostics
        );
        let workflow = diagnosis.workflow.unwrap();
        let restored: WorkflowDefinitionYaml =
            serde_json::from_value(serde_json::to_value(&workflow).unwrap()).unwrap();
        assert_eq!(restored, workflow);
        assert_eq!(
            restored
                .node_by_name("main")
                .unwrap()
                .requires_approval_completion(),
            approval
        );
    }
}

#[test]
fn test_delegate定義_必須fieldと未知キーと回数値域を拒否する() {
    // Given
    let delegate = definition()["nodes"]["main"]["completion"]["delegate"].clone();
    let mut cases = vec![json!(null), json!("verify"), json!({}), json!([])];
    for field in ["child", "when", "max_iterations"] {
        let mut invalid = delegate.clone();
        invalid.as_object_mut().unwrap().remove(field);
        cases.push(invalid);
    }
    for count in [json!(-1), json!(1.5), json!("2"), json!(null)] {
        let mut invalid = delegate.clone();
        invalid["max_iterations"] = count;
        cases.push(invalid);
    }
    for (field, value) in [
        ("unknown", json!(true)),
        ("when", json!({"and": []})),
        ("when", json!({"or": ["done", {"and": []}]})),
        ("inputs", json!(42)),
    ] {
        let mut invalid = delegate.clone();
        invalid[field] = value;
        cases.push(invalid);
    }
    for invalid in cases {
        let mut value = definition();
        value["nodes"]["main"]["completion"]["delegate"] = invalid;
        // When
        let diagnosis = check(&value);
        // Then
        assert!(diagnosis.workflow.is_none());
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|item| item.code == "WFS002"),
            "{:?}",
            diagnosis.diagnostics
        );
    }
}

#[test]
fn test_delegate定義_宣言するnodeとchildの条件を検査する() {
    // Given
    let mut cases = Vec::new();
    for kind in [
        json!({"command": "check"}),
        json!({"fanout": {"children": ["extra"]}}),
        json!({"sequence": {"children": ["extra"]}}),
    ] {
        let mut value = definition();
        value["nodes"]["main"]
            .as_object_mut()
            .unwrap()
            .remove("session");
        value["nodes"]["main"]
            .as_object_mut()
            .unwrap()
            .extend(kind.as_object().unwrap().clone());
        value["nodes"]["extra"] = json!({"command": "true"});
        cases.push((value, "WFC011"));
    }
    for isolated in [false, true] {
        let mut value = definition();
        value["nodes"]["main"]
            .as_object_mut()
            .unwrap()
            .remove("artifact");
        if isolated {
            value["nodes"]["main"]["worktree"] = json!("isolated");
        }
        cases.push((value, "WFT006"));
    }
    for child in ["unknown", "main"] {
        let mut value = definition();
        value["nodes"]["main"]["completion"]["delegate"]["child"] = json!(child);
        cases.push((
            value,
            if child == "unknown" {
                "WFR001"
            } else {
                "WFC008"
            },
        ));
    }
    let mut missing_artifact = definition();
    missing_artifact["nodes"]["verify"] =
        json!({"session": {"provider": "claude", "facets": {"instruction": "check"}}});
    cases.push((missing_artifact, "WFT006"));
    let mut cycle = definition();
    cycle["nodes"]["verify"] = json!({"sequence": {"children": ["main"]}});
    cases.push((cycle, "WFC008"));
    for (value, expected_code) in cases {
        // When
        let diagnosis = check(&value);
        // Then
        assert!(diagnosis.has_errors(), "{value}");
        assert!(super::super::storage::parse_workflow_source(
            &value.to_string(),
            std::path::Path::new(".")
        )
        .is_err());
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|item| item.code == expected_code),
            "{:?}",
            diagnosis.diagnostics
        );
    }
}

#[test]
fn test_delegate定義_child共有を拒否しdelegateだけからの参照を到達可能にする() {
    // Given
    assert!(check(&definition()).diagnostics.is_empty());
    for other_delegate in [false, true] {
        let mut value = definition();
        value["nodes"]["worker"] = value["nodes"]["main"].clone();
        value["nodes"]["main"] = if other_delegate {
            value["nodes"]["other"] = value["nodes"]["worker"].clone();
            json!({"sequence": {"children": ["worker", "other"]}})
        } else {
            json!({"sequence": {"children": ["worker", "verify"]}})
        };
        // When
        let diagnosis = check(&value);
        // Then
        assert!(diagnosis.has_errors());
        assert!(super::super::storage::parse_workflow_source(
            &value.to_string(),
            std::path::Path::new(".")
        )
        .is_err());
        assert!(diagnosis
            .diagnostics
            .iter()
            .any(|item| item.code == "WFC006"));
    }
}

#[test]
fn test_delegate定義_childの全kindのschemaと多段参照を検査する() {
    // Given
    for (kind, path) in [
        (
            json!({"command": "check", "artifact": "verdict"}),
            "child.passed",
        ),
        (
            json!({"session": {"provider": "claude", "facets": {"instruction": "check"}}, "artifact": "verdict"}),
            "child.passed",
        ),
        (
            json!({"sequence": {"children": ["judge"]}}),
            "child.judge.passed",
        ),
        (
            json!({"fanout": {"children": ["judge"]}}),
            "child.judge.passed",
        ),
        (
            json!({"fanout": {"items": [1, 2], "children": [{"judge": {"inputs": {"item": "items"}}}]}}),
            "child.1.passed",
        ),
    ] {
        let mut value = definition();
        value["nodes"]["verify"] = kind;
        if path != "child.passed" {
            value["nodes"]["judge"] =
                json!({"command": "check", "artifact": "verdict", "input": ["item"]});
        }
        value["nodes"]["main"]["completion"]["delegate"]["when"] = json!(path);
        // When / Then
        assert!(
            check(&value).diagnostics.is_empty(),
            "{:?}",
            check(&value).diagnostics
        );
        for invalid in [path.replace("passed", "missing"), format!("{path}.field")] {
            value["nodes"]["main"]["completion"]["delegate"]["when"] = json!(invalid);
            assert!(check(&value)
                .diagnostics
                .iter()
                .any(|item| item.code == "WFT001"));
        }
    }
}

#[test]
fn test_delegate定義_optionalと非booleanを拒否しchild予約はdelegate親だけに適用する() {
    // Given
    for schema in [
        json!({"type": "object", "properties": {"passed": {"type": "boolean"}}, "required": []}),
        json!({"type": "object", "properties": {"passed": "string"}, "required": ["passed"]}),
    ] {
        let mut value = definition();
        value["schemas"]["verdict"] = schema;
        // When / Then
        assert!(check(&value)
            .diagnostics
            .iter()
            .any(|item| item.code == "WFT001"));
    }
    let mut value = definition();
    value["schemas"]["parent-result"]["properties"]["child"] = json!("string");
    assert!(check(&value)
        .diagnostics
        .iter()
        .any(|item| item.code == "WFT005"));
    value["nodes"]["main"]
        .as_object_mut()
        .unwrap()
        .remove("completion");
    value["nodes"].as_object_mut().unwrap().remove("verify");
    assert!(check(&value).diagnostics.is_empty());
}

#[test]
fn test_delegate定義_luaは従来どおりdelegateを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for completion in [
        "{ delegate = 'worker' }",
        "{ require = r.completion.approval, delegate = 'worker' }",
        "{ require = r.completion.approval, extra = true }",
    ] {
        let source = format!("local r = require('releash')\nreturn r.workflow{{ name = 'delegate', description = 'test', main = r.command{{ command = 'true', completion = {completion} }} }}");
        // When
        let diagnosis = diagnose_lua_workflow_source(
            "delegate.lua",
            &source,
            directory.path(),
            directory.path(),
            None,
        );
        // Then
        assert!(diagnosis.workflow.is_none());
        assert_eq!(diagnosis.diagnostics.len(), 1);
        assert_eq!(diagnosis.diagnostics[0].code, "WFS002");
        assert_eq!(diagnosis.diagnostics[0].stage, DiagnosticStage::ParseShape);
        assert_eq!(diagnosis.diagnostics[0].severity, Severity::Error);
        assert_eq!(
            diagnosis.diagnostics[0].field.as_deref(),
            Some("completion")
        );
        assert_eq!(
            diagnosis.diagnostics[0].message,
            "completion map only accepts the key 'require'"
        );
    }
}

#[test]
fn test_delegate配線_親inputと親artifactとrequestを受理し名前と参照を検査する() {
    // Given
    for source in ["spec", "main.task", "request"] {
        let mut value = definition();
        value["nodes"]["main"]["input"] = json!([{"spec": "text"}]);
        value["nodes"]["verify"]["input"] = json!([{"task": "text"}]);
        value["nodes"]["main"]["completion"]["delegate"]["inputs"] = json!({"task": source});
        // When / Then
        assert!(
            check(&value).diagnostics.is_empty(),
            "{:?}",
            check(&value).diagnostics
        );
        for invalid in [
            json!({"unknown": "request"}),
            json!({"task": "main.unknown"}),
            json!({"task": "items"}),
            json!({"task": " main.task"}),
            json!({"task": "request.field"}),
        ] {
            value["nodes"]["main"]["completion"]["delegate"]["inputs"] = invalid;
            assert!(check(&value).has_errors(), "{value}");
            assert!(super::super::storage::parse_workflow_source(
                &value.to_string(),
                std::path::Path::new(".")
            )
            .is_err());
        }
    }
    let mut ambiguous = definition();
    ambiguous["nodes"]["main"]["input"] = json!(["main"]);
    ambiguous["nodes"]["verify"]["input"] = json!(["task"]);
    ambiguous["nodes"]["main"]["completion"]["delegate"]["inputs"] = json!({"task": "main"});
    assert!(check(&ambiguous)
        .diagnostics
        .iter()
        .any(|item| item.code == "WFR008"));
}

#[test]
fn test_delegate定義_親sessionと合成子との包含循環を拒否する() {
    // Given
    let mut value = definition();
    let parent = value["nodes"]
        .as_object_mut()
        .unwrap()
        .remove("main")
        .unwrap();
    value["nodes"]["worker"] = parent;
    value["nodes"]["main"] = json!({"sequence": {"children": ["worker"]}});
    // When / Then
    assert!(
        check(&value).diagnostics.is_empty(),
        "{:?}",
        check(&value).diagnostics
    );
    value["nodes"]["verify"] = json!({"sequence": {"children": ["worker"]}});
    let diagnosis = check(&value);
    assert!(diagnosis.has_errors());
    let workflow = diagnosis.workflow.unwrap();
    let errors = crate::domain::workflow::services::validation::validate_all(&workflow);
    assert!(errors.iter().any(|error| matches!(error, crate::domain::workflow::services::validation::ValidationError::CompositeInclusionCycle { .. })));
}

#[test]
fn test_delegate配線_異なるcontractの親artifactと合成子mapも受理する() {
    // Given
    let mut value = definition();
    value["schemas"]["composed"] = json!({"type": "object", "properties": {"task": "string", "child": value["schemas"]["verdict"].clone()}, "required": ["task", "child"]});
    value["nodes"]["verify"]["input"] = json!([{"result": "composed"}]);
    value["nodes"]["main"]["completion"]["delegate"]["inputs"] = json!({"result": "main"});
    // When / Then
    assert!(
        check(&value).diagnostics.is_empty(),
        "{:?}",
        check(&value).diagnostics
    );
    value["schemas"]["composed"]["properties"]["child"]["properties"]["passed"] = json!("string");
    assert!(
        check(&value).diagnostics.is_empty(),
        "{:?}",
        check(&value).diagnostics
    );

    value = definition();
    value["nodes"]["verify"] =
        json!({"sequence": {"children": ["check"]}, "input": [{"result": "flag"}]});
    value["nodes"]["check"] = json!({"command": "check", "artifact": "verdict"});
    value["nodes"]["main"]["completion"]["delegate"]["when"] = json!("child.check.passed");
    value["nodes"]["main"]["completion"]["delegate"]["inputs"] = json!({"result": "main.child"});
    assert!(
        check(&value).diagnostics.is_empty(),
        "{:?}",
        check(&value).diagnostics
    );
    value["schemas"]["checks"] = json!({"type": "object", "properties": {"check": value["schemas"]["verdict"].clone()}, "required": ["check"]});
    value["nodes"]["verify"]["input"] = json!([{"result": "checks"}]);
    assert!(
        check(&value).diagnostics.is_empty(),
        "{:?}",
        check(&value).diagnostics
    );
}

#[test]
fn test_delegate定義_ゼロ回と意味的違反を構造化して分類する() {
    // Given
    for (mutation, code, stage, field) in [
        (
            "zero",
            "WFC005",
            DiagnosticStage::ControlFlow,
            "completion.delegate.max_iterations",
        ),
        (
            "kind",
            "WFC011",
            DiagnosticStage::ControlFlow,
            "completion.delegate",
        ),
        (
            "artifact",
            "WFT006",
            DiagnosticStage::Typecheck,
            "completion.delegate",
        ),
        (
            "child",
            "WFT006",
            DiagnosticStage::Typecheck,
            "completion.delegate.child",
        ),
    ] {
        let mut value = definition();
        match mutation {
            "zero" => value["nodes"]["main"]["completion"]["delegate"]["max_iterations"] = json!(0),
            "kind" => {
                value["nodes"]["main"]
                    .as_object_mut()
                    .unwrap()
                    .remove("session");
                value["nodes"]["main"]["command"] = json!("true");
            }
            "artifact" => {
                value["nodes"]["main"]
                    .as_object_mut()
                    .unwrap()
                    .remove("artifact");
            }
            "child" => {
                value["nodes"]["verify"] =
                    json!({"session": {"provider": "codex", "facets": {"instruction": "check"}}})
            }
            _ => unreachable!(),
        }
        // When
        let diagnosis = check(&value);
        // Then
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|d| d.code == code && d.stage == stage && d.field.as_deref() == Some(field)),
            "{:?}",
            diagnosis.diagnostics
        );
    }
}

#[test]
fn test_delegate配線_異なるcontractの型付き親inputを受理する() {
    // Given
    let mut value = definition();
    value["nodes"]["main"]["input"] = json!([{"spec": "text"}]);
    value["nodes"]["verify"]["input"] = json!([{"task": "flag"}]);
    value["nodes"]["main"]["completion"]["delegate"]["inputs"] = json!({"task": "spec"});
    // When / Then
    assert!(
        check(&value).diagnostics.is_empty(),
        "{:?}",
        check(&value).diagnostics
    );
}

#[test]
fn test_delegate定義_述語型エラーは辺でなくsessionの宣言位置を指す() {
    // Given
    let source = "name: delegate\ndescription: test\nschemas:\n  result: {type: object, properties: {done: {type: boolean}}, required: [done]}\nnodes:\n  main:\n    sequence:\n      children:\n        - worker:\n            rules:\n              - when: {on: done, then: end}\n                next: end\n        - end: {command: true}\n  worker:\n    session: {provider: codex, facets: {instruction: implement}}\n    artifact: result\n    completion:\n      delegate:\n        child: verify\n        when: child.stdout\n        max_iterations: 1\n  verify: {command: check}\n".replace("command: true", "command: 'true'");
    // When
    let diagnosis = diagnose_workflow_source(&source, None);
    // Then
    let diagnostic = diagnosis
        .diagnostics
        .iter()
        .find(|d| d.code == "WFT001")
        .unwrap_or_else(|| panic!("{:?}", diagnosis.diagnostics));
    assert_eq!(
        diagnostic.field.as_deref(),
        Some("completion.delegate.when")
    );
    assert!(diagnostic.message.contains("completion.delegate.when"));
    assert!(!diagnostic.message.contains("rules"));
    let span = super::super::span_map::YamlSpanMap::parse(&source).unwrap();
    assert_eq!(
        diagnostic.span,
        span.field_span("nodes.worker.completion.delegate.when")
    );
}

#[test]
fn test_delegate定義_共有と包含循環の文言はsessionをcompositeと呼ばない() {
    // Given
    let mut value = definition();
    value["nodes"]["worker"] = value["nodes"]["main"].clone();
    value["nodes"]["main"] = json!({"sequence": {"children": ["worker", "verify"]}});
    // When / Then
    let diagnosis = check(&value);
    let errors: Vec<_> = diagnosis
        .diagnostics
        .iter()
        .filter(|d| d.code == "WFC006")
        .collect();
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .all(|d| !d.message.contains("composite node 'worker'")),
        "{errors:?}"
    );
    value["nodes"]["verify"] = json!({"sequence": {"children": ["worker"]}});
    let diagnosis = check(&value);
    let errors: Vec<_> = diagnosis
        .diagnostics
        .iter()
        .filter(|d| d.code == "WFC008")
        .collect();
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .all(|d| !d.message.contains("composite node 'worker'")),
        "{errors:?}"
    );
}

#[test]
fn test_delegate正本サンプル_itemsの要素contractを実装sessionのtaskと照合する() {
    // Given
    let source = include_str!("../../../../../workflows/examples/full-cycle-development.yml");
    assert!(diagnose_workflow_source(source, None)
        .diagnostics
        .is_empty());
    let source = source.replacen(
        "    - task: implement-task",
        "    - task: implement-task-result",
        1,
    );
    // When
    let diagnosis = diagnose_workflow_source(&source, None);
    // Then
    assert!(
        diagnosis.diagnostics.iter().any(|d| d.code == "WFT003"),
        "{:?}",
        diagnosis.diagnostics
    );
}

#[test]
fn test_delegate定義_下流配線は親のchild合成schemaを参照できる() {
    // Given
    let mut value = definition();
    value["nodes"]["implement"] = value["nodes"]["main"].clone();
    value["nodes"]["main"] = json!({"sequence": {"children": ["implement", {"done": {"inputs": {"verdict": "implement.child.passed"}}}]}});
    value["nodes"]["done"] = json!({"command": "done", "input": ["verdict"]});
    // When / Then
    assert!(
        check(&value).diagnostics.is_empty(),
        "{:?}",
        check(&value).diagnostics
    );
    value["nodes"]["main"]["sequence"]["children"][1]["done"]["inputs"]["verdict"] =
        json!("implement.child.unknown");
    assert!(check(&value).diagnostics.iter().any(|d| d.code == "WFR007"));
}

#[test]
fn test_delegate定義_enumをwhenに使う型エラーはdelegateの宣言を指す() {
    // Given
    let mut value = definition();
    value["schemas"]["verdict"]["properties"]["passed"] =
        json!({"type": "string", "enum": ["yes", "no"]});
    // When
    let diagnosis = check(&value);
    // Then
    let diagnostic = diagnosis
        .diagnostics
        .iter()
        .find(|item| item.code == "WFT001")
        .unwrap();
    assert_eq!(
        diagnostic.field.as_deref(),
        Some("completion.delegate.when")
    );
    assert_eq!(diagnostic.message, "node 'main' completion.delegate.when: completion.delegate.when field 'child.passed' must be a required boolean");
    assert!(!diagnostic.message.contains("when.on"));
}

#[test]
fn test_delegate定義_artifactを省略したisolated_sessionのchildを受理する() {
    // Given
    let mut value = definition();
    value["nodes"]["verify"] = json!({"session": {"provider": "codex", "facets": {"instruction": "verify"}}, "worktree": "isolated"});
    value["nodes"]["main"]["completion"]["delegate"]["when"] = json!("done");
    // When
    let diagnosis = check(&value);
    // Then
    assert!(
        diagnosis.diagnostics.is_empty(),
        "{:?}",
        diagnosis.diagnostics
    );
    assert!(diagnosis.workflow.is_some());
}

#[test]
fn test_delegate定義_非空inputsは定義の保存復元で配線を保つ() {
    // Given
    let mut value = definition();
    value["nodes"]["main"]["input"] = json!(["spec"]);
    value["nodes"]["verify"]["input"] = json!(["spec", "task", "previous", "requested"]);
    let inputs = json!({"spec": "spec", "task": "main.task", "previous": "main.child.passed", "requested": "request"});
    value["nodes"]["main"]["completion"]["delegate"]["inputs"] = inputs.clone();
    let diagnosis = check(&value);
    assert!(
        diagnosis.diagnostics.is_empty(),
        "{:?}",
        diagnosis.diagnostics
    );
    let workflow = diagnosis.workflow.unwrap();
    // When
    let serialized = serde_json::to_value(&workflow).unwrap();
    let restored: WorkflowDefinitionYaml = serde_json::from_value(serialized.clone()).unwrap();
    // Then
    assert_eq!(
        serialized["nodes"]["main"]["completion"]["delegate"]["inputs"],
        inputs
    );
    assert_eq!(restored, workflow);
}
