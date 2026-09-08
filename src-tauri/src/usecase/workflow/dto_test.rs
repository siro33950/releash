use super::*;

#[test]
fn test_述語の定義dto_全ての参照と論理構造と遷移先を保持する() {
    // Given
    for on in [
        serde_json::json!("passed"),
        serde_json::json!({"and": ["passed", "details.passed"]}),
        serde_json::json!({"or": ["passed", "clean"]}),
        serde_json::json!({"and": ["passed", {"or": ["clean", "skipped"]}]}),
    ] {
        let source =
            include_str!("../../adaptor/gateway/workflow/fixtures/valid/predicate-routing.yml")
                .replace("{and: [passed, {or: [clean, skipped]}]}", &on.to_string());
        let definition: domain::WorkflowDefinition = serde_saphyr::from_str(&source).unwrap();
        // When
        let value = serde_json::to_value(workflow_to_dto(&definition)).unwrap();
        // Then
        let main = value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["name"] == "main")
            .unwrap();
        assert_eq!(
            main["sequence"]["children"][0]["rules"][0],
            serde_json::json!({"type": "when", "on": on, "then": "done", "next": "fix"})
        );
    }
}

#[test]
fn test_辺の定義dto_switchとloopguardとnextの意味を保持する() {
    // Given
    let rules = [
        (
            domain::Rule::Switch {
                on: "verdict".into(),
                cases: BTreeMap::from([("SHIP".into(), "done".into())]),
                next: Some("fix".into()),
            },
            serde_json::json!({"type": "switch", "on": "verdict", "cases": {"SHIP": "done"}, "next": "fix"}),
        ),
        (
            domain::Rule::LoopGuard {
                max_iterations: 3,
                on_exhausted: "fix".into(),
            },
            serde_json::json!({"type": "loop_guard", "max_iterations": 3, "on_exhausted": "fix"}),
        ),
        (
            domain::Rule::Next("done".into()),
            serde_json::json!({"type": "next", "next": "done"}),
        ),
    ];
    for (rule, expected) in rules {
        // When
        let value = serde_json::to_value(rule_to_dto(&rule)).unwrap();
        // Then
        assert_eq!(value, expected);
    }
}

#[test]
fn test_completion表示dto_承認要求はmapで示し要求なしは省略する() {
    // Given
    for (completion, expected) in [
        (domain::NodeCompletion::default(), None),
        (
            domain::NodeCompletion::require_approval(),
            Some(serde_json::json!({"require": "approval"})),
        ),
    ] {
        let node = domain::NodeDefinition {
            completion,
            ..Default::default()
        };
        // When
        let dto = node_to_dto(&node);
        let value = serde_json::to_value(&dto).unwrap();
        // Then
        assert_eq!(value.get("completion"), expected.as_ref());
        assert_eq!(
            serde_json::from_value::<NodeDefinitionDto>(value).unwrap(),
            dto
        );
    }
}
