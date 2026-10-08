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

#[test]
fn test_delegate表示dto_配線と述語と上限を含むcompletionを保持する() {
    // Given
    let workflow: domain::WorkflowDefinition = serde_saphyr::from_str("name: test\ndescription: test\nnodes:\n  main: {session: {provider: codex}, artifact: result, completion: {require: approval, delegate: {child: check, inputs: {task: main.task}, when: child.ok, max_iterations: 2}}}\n  check: {command: check}").unwrap();
    // When
    let dto = node_to_dto(workflow.node_by_name("main").unwrap());
    let value = serde_json::to_value(&dto).unwrap();
    // Then
    assert_eq!(value["completion"]["require"], "approval");
    assert_eq!(value["completion"]["delegate"]["child"], "check");
    assert_eq!(value["completion"]["delegate"]["when"], "child.ok");
    assert_eq!(value["completion"]["delegate"]["max_iterations"], 2);
    assert_eq!(
        value["completion"]["delegate"]["inputs"][0],
        serde_json::json!({"parameter": "task", "source": "main.task"})
    );
    assert_eq!(
        serde_json::from_value::<NodeDefinitionDto>(value).unwrap(),
        dto
    );
}
pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_workflow出力_既存の転送形式で直列化する() {
        // Given
        let workflow = WorkflowDto {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            source_format: WorkflowSourceFormatDto::Yaml,
            schemas: [(
                "plan".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            )]
            .into_iter()
            .collect(),
            nodes: vec![NodeDefinitionDto {
                name: "node".to_string(),
                kind: NodeKindDto::Session,
                session: Some(SessionSpecDto {
                    provider: AgentSessionProviderDto::Claude,
                    model: None,
                    permission: None,
                    facets: FacetRefsDto {
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                }),
                artifact: Some("plan".to_string()),
                input: vec![InputParamDto {
                    name: "item".to_string(),
                    contract: Some("plan".to_string()),
                }],
                ..Default::default()
            }],
        };

        // When
        let actual = serde_json::to_value(workflow).unwrap();

        // Then
        assert_eq!(
            actual,
            serde_json::json!({
                "name": "wf",
                "description": "desc",
                "builtin": false,
                "sourceFormat": "yaml",
                "schemas": {
                    "plan": {
                        "type": "object",
                        "properties": {},
                        "required": []
                    }
                },
                "nodes": [{
                    "name": "node",
                    "kind": "session",
                    "session": {
                        "provider": "claude",
                        "facets": {
                            "instruction": "inst"
                        }
                    },
                    "artifact": "plan",
                    "input": [{"name": "item", "contract": "plan"}]
                }]
            })
        );
    }

    #[test]
    fn workflow_dto_exposes_lua_only_as_definition_source_metadata() {
        let workflow = domain::WorkflowDefinition {
            name: "lua-workflow".to_string(),
            description: "Lua".to_string(),
            ..domain::WorkflowDefinition::default()
        };

        let value = serde_json::to_value(workflow_to_dto_with_source_format(
            &workflow,
            domain::WorkflowSourceFormat::Lua,
        ))
        .unwrap();

        assert_eq!(value["sourceFormat"], "lua");
        assert!(serde_json::to_value(workflow)
            .unwrap()
            .get("sourceFormat")
            .is_none());
    }

    #[test]
    fn workflow_to_dto_maps_knowledge_refs_to_ordered_json_array() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            nodes: vec![domain::NodeDefinition {
                name: "review".to_string(),
                kind: domain::NodeKind::Session(domain::SessionSpec {
                    provider: crate::domain::provider_lifecycle::ProviderKind::Codex,
                    permission: Some(domain::SessionPermission::ReadOnly),
                    facets: domain::FacetRefs {
                        knowledge: vec!["knowledge-a".to_string(), "knowledge-b".to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            entry: "review".to_string(),
            ..Default::default()
        };

        let dto = workflow_to_dto(&definition);

        assert_eq!(
            dto.nodes[0].session.as_ref().unwrap().facets.knowledge,
            vec!["knowledge-a", "knowledge-b"]
        );
        assert_eq!(
            serde_json::to_value(dto).unwrap()["nodes"][0]["session"]["facets"]["knowledge"],
            serde_json::json!(["knowledge-a", "knowledge-b"])
        );
        assert_eq!(
            serde_json::to_value(workflow_to_dto(&definition)).unwrap()["nodes"][0]["session"]
                ["provider"],
            serde_json::json!("codex")
        );
        assert_eq!(
            serde_json::to_value(workflow_to_dto(&definition)).unwrap()["nodes"][0]["session"]
                ["permission"],
            serde_json::json!("read-only")
        );
    }

    #[test]
    fn workflow_to_dto_preserves_loop_guard() {
        let definition = domain::WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            nodes: vec![
                domain::NodeDefinition {
                    name: "main".to_string(),
                    kind: domain::NodeKind::Sequence(domain::SequenceSpec {
                        entry: None,
                        children: vec![domain::ChildEntry {
                            name: "fix".to_string(),
                            inputs: Vec::new(),
                            rules: Some(vec![domain::Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "done".to_string(),
                            }]),
                        }],
                    }),
                    ..Default::default()
                },
                domain::NodeDefinition {
                    name: "fix".to_string(),
                    ..Default::default()
                },
            ],
            entry: "main".to_string(),
            ..Default::default()
        };

        let dto = workflow_to_dto(&definition);

        assert_eq!(
            serde_json::to_value(dto).unwrap()["nodes"][0]["sequence"]["children"][0]["rules"][0],
            serde_json::json!({
                "type": "loop_guard",
                "max_iterations": 2,
                "on_exhausted": "done"
            })
        );
    }

    #[test]
    fn fanout_spec_dto_serializes_child_and_items_sources() {
        let literal = FanoutSpecDto {
            children: vec![ChildEntryDto {
                name: "review".to_string(),
                inputs: Vec::new(),
                rules: None,
            }],
            items: Some(ItemsSourceDto::Literal(vec![serde_json::json!({
                "thread_id": "thread-1"
            })])),
        };
        assert_eq!(
            serde_json::to_value(literal).unwrap(),
            serde_json::json!({
                "children": [{"name": "review"}],
                "items": [{"thread_id": "thread-1"}]
            })
        );

        let reference = FanoutSpecDto {
            children: vec![
                ChildEntryDto {
                    name: "review-opus".to_string(),
                    inputs: Vec::new(),
                    rules: None,
                },
                ChildEntryDto {
                    name: "review-gpt".to_string(),
                    inputs: Vec::new(),
                    rules: None,
                },
            ],
            items: Some(ItemsSourceDto::ArtifactField("scan.threads".to_string())),
        };
        assert_eq!(
            serde_json::to_value(reference).unwrap(),
            serde_json::json!({
                "children": [{"name": "review-opus"}, {"name": "review-gpt"}],
                "items": "scan.threads"
            })
        );
    }
}
