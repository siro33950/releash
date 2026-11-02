use super::*;

#[test]
fn test_workflow設定入力_転送値の真偽と欠落を入力データへ写す() {
    // Given
    let values = [(Some(true), true), (Some(false), false), (None, false)];

    // When
    for (wire_value, expected) in values {
        let input =
            crate::usecase::app_config::WorkflowConfigInput::try_from(wire::WorkflowSection {
                approval_auto_approve: wire_value.map(|value| value.try_into().unwrap()),
            })
            .unwrap();
        // Then
        assert_eq!(input.approval_auto_approve, expected);
    }
}

#[test]
fn test_review転送_解決済みthreadの全項目を既存のjsonの形で保持する() {
    use crate::domain::comment::{Author, Comment, Resolve, State, Target, Thread};
    // Given
    let actor = Author::provider_agent("codex".into(), Some("session".into()));
    let thread = Thread {
        id: "thread".into(),
        worktree_name: "/repo".into(),
        target: Target {
            file_path: Some("src/main.rs".into()),
            line_number: Some(3),
            end_line: Some(5),
            side: None,
            end_side: None,
        },
        state: State::Resolved,
        comments: vec![Comment {
            id: "comment".into(),
            thread_id: "thread".into(),
            author: actor.clone(),
            content: "content".into(),
            created_at: 1.0,
        }],
        resolve: Some(Resolve {
            actor,
            outcome: "fixed".into(),
            summary: "summary".into(),
            resolved_at: 2.0,
        }),
        created_at: 1.0,
        updated_at: 2.0,
        version: 3,
        can_resolve: false,
    };
    // When
    let message = wire::ReviewThreadDto::try_from(thread).unwrap();
    let json = wire::from_message("releash.client.v1.ReviewThreadDto", &message).unwrap();
    // Then
    let actor = serde_json::json!({"kind":"agent", "backendId":"codex", "model":null, "displayName":"codex"});
    assert_eq!(
        json,
        serde_json::json!({
            "id":"thread", "worktreeName":"/repo", "author":actor,
            "target":{"filePath":"src/main.rs", "lineNumber":3, "endLine":5}, "state":"resolved",
            "comments":[{"id":"comment", "threadId":"thread", "author":actor, "content":"content", "createdAt":1.0}],
            "resolve":{"actor":actor, "outcome":"fixed", "summary":"summary", "resolvedAt":2.0},
            "createdAt":1.0, "updatedAt":2.0, "version":3, "canResolve":false,
        })
    );
}

#[test]
fn test_review転送_最初のcommentの作者を保持しcommentがなければ拒否する() {
    use crate::domain::comment::{Author, Comment, State, Target, Thread};
    // Given
    let mut agent = Author::provider_agent("codex".into(), Some("private-session".into()));
    agent.model = Some("model".into());
    let mut thread = Thread {
        id: "thread".into(),
        worktree_name: "/repo".into(),
        target: Target {
            file_path: None,
            line_number: None,
            end_line: None,
            side: None,
            end_side: None,
        },
        state: State::Open,
        comments: vec![
            Comment {
                id: "first".into(),
                thread_id: "thread".into(),
                author: Author::human(),
                content: "first".into(),
                created_at: 1.0,
            },
            Comment {
                id: "reply".into(),
                thread_id: "thread".into(),
                author: agent,
                content: "reply".into(),
                created_at: 2.0,
            },
        ],
        resolve: None,
        created_at: 1.0,
        updated_at: 2.0,
        version: 2,
        can_resolve: true,
    };
    // When
    let message = wire::ReviewThreadDto::try_from(thread.clone()).unwrap();
    let json = wire::from_message("releash.client.v1.ReviewThreadDto", &message).unwrap();
    thread.comments.clear();
    let empty = wire::ReviewThreadDto::try_from(thread);
    // Then
    let human =
        serde_json::json!({"kind":"human", "backendId":null, "model":null, "displayName":"Human"});
    assert_eq!(json["author"], human);
    assert_eq!(json["comments"][0]["author"], human);
    assert_eq!(
        json["comments"][1]["author"],
        serde_json::json!({"kind":"agent", "backendId":"codex", "model":"model", "displayName":"codex"})
    );
    assert_eq!(empty.unwrap_err(), "Missing first review comment");
}

#[test]
fn test_pane転送_分割と全タブを往復して欠落と未知の種類を拒否する() {
    use crate::domain::workspace_state::value_objects::pane_layout::{
        PaneLayout, PaneTab, PaneTabKind, SplitAxis,
    };
    let pane = PaneLayout::pane(
        "a".into(),
        vec![
            PaneTab {
                agent_session_id: None,
                node_id: None,
                id: "terminal".into(),
                kind: PaneTabKind::Terminal,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
            PaneTab {
                agent_session_id: None,
                node_id: None,
                id: "workflow".into(),
                kind: PaneTabKind::Command,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
            PaneTab {
                agent_session_id: None,
                node_id: None,
                id: "file".into(),
                kind: PaneTabKind::File,
                file_path: Some("src/file.rs".into()),
                file_mode: Some(crate::domain::code::value_objects::review::FileMode::Diff),
                file_base: Some(crate::domain::code::ReviewBase::BranchBase),
            },
            PaneTab {
                agent_session_id: Some("agent-session".into()),
                node_id: None,
                id: "session".into(),
                kind: PaneTabKind::Session,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
            PaneTab {
                agent_session_id: None,
                node_id: Some("command-node".into()),
                id: "command".into(),
                kind: PaneTabKind::Command,
                file_path: None,
                file_mode: None,
                file_base: None,
            },
        ],
        Some("file".into()),
    )
    .unwrap();
    for axis in [SplitAxis::Horizontal, SplitAxis::Vertical] {
        let layout = PaneLayout::split(
            "split".into(),
            axis,
            0.7,
            pane.clone(),
            PaneLayout::pane("b".into(), vec![], None).unwrap(),
        )
        .unwrap();
        let message = wire::PaneLayout::try_from(layout.clone()).unwrap();
        assert_eq!(PaneLayout::try_from(message).unwrap(), layout);
    }
    let mut state = crate::domain::workspace_state::WorkspaceState {
        version: 1,
        panes: None,
        tabs: crate::domain::workspace_state::value_objects::WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: crate::domain::workspace_state::value_objects::WorkspaceLayoutState {
            center_tab: "agent".into(),
            active_view: "git".into(),
            left_nav_collapsed: true,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };
    state.panes = Some(pane.clone());
    let message = wire::WorkspaceStateDto::try_from(state.clone()).unwrap();
    assert!(message.pane_layout.is_some());
    assert_eq!(
        crate::domain::workspace_state::WorkspaceState::try_from(message).unwrap(),
        state
    );
    assert!(PaneLayout::try_from(wire::PaneLayout::default()).is_err());
    let mut message = wire::PaneLayout::try_from(pane).unwrap();
    for kind in [2, 999] {
        if let Some(wire::pane_layout::Node::Pane(pane)) = &mut message.node {
            pane.tabs[0].kind = kind;
        }
        assert!(PaneLayout::try_from(message.clone()).is_err());
    }
    for axis in [0, 999, wire::SplitAxis::Horizontal as i32] {
        assert!(PaneLayout::try_from(wire::PaneLayout {
            node: Some(wire::pane_layout::Node::Split(Box::new(wire::PaneSplit {
                axis,
                ..Default::default()
            })))
        })
        .is_err());
    }
}

#[test]
fn test_行の側_proto変換で前後を保持し不明な値を拒否する() {
    use crate::domain::comment::LineSide as ReviewLineSide;
    // Given / When / Then
    for side in [ReviewLineSide::Additions, ReviewLineSide::Deletions] {
        let wire = wire::ReviewLineSideDto::try_from(side).unwrap();
        assert_eq!(ReviewLineSide::try_from(wire).unwrap(), side);
    }
    for value in [None, Some(99)] {
        assert!(ReviewLineSide::try_from(wire::ReviewLineSideDto { value }).is_err());
    }
}

#[test]
fn test_session表示状態_全状態をwireのenumへ変換する() {
    use crate::domain::agent_session::aggregates::AgentSessionAvailability;
    use wire::AgentSessionAvailability as Value;
    // Given / When / Then
    for (availability, value) in [
        (AgentSessionAvailability::Open, Value::Open),
        (AgentSessionAvailability::Paused, Value::Paused),
        (
            AgentSessionAvailability::Indeterminate,
            Value::Indeterminate,
        ),
        (AgentSessionAvailability::Archived, Value::Archived),
    ] {
        assert_eq!(wire::AgentSessionAvailability::from(availability), value);
    }
}

#[test]
fn test_node詳細の転送_分類と実行識別とmodelと過去の試行の値を保持する() {
    use crate::usecase::workflow::*;
    // Given
    let detail = WorkspaceNodeDetailDto {
        classification: "attention".into(),
        node_execution_id: Some("visit".into()),
        node_name: Some("review".into()),
        model: Some("test-model".into()),
        past_attempts: vec![WorkspaceNodeAttemptDto {
            id: "previous".into(),
            title: "Previous review".into(),
            updated_at: 123.5,
            error_reason: Some("Failed review".into()),
        }],
        execution_id: Some("execution".into()),
        can_abort: true,
        process_presence: "confirmed_absent",
        worktree: None,
        id: "current".into(),
        title: "Review".into(),
        status: "waiting".into(),
        submit_received: false,
        stop_received: false,
        waiting_for: None,
        has_artifact: true,
        error_reason: None,
        capabilities: WorkspaceNodeCapabilitiesDto {
            can_rename: false,
            can_approve: true,
            can_retry: false,
            can_resume_session: false,
        },
        updated_at: 124.0,
        content: WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
            session_id: None,
        }),
    };
    // When
    let wire = wire::WorkspaceNodeDetailDto::try_from(detail).unwrap();
    // Then
    assert_eq!(
        wire.classification.unwrap().value,
        Some(wire::workspace_status_classification::Value::Attention as i32)
    );
    assert_eq!(wire.node_execution_id.as_deref(), Some("visit"));
    assert_eq!(wire.node_name.as_deref(), Some("review"));
    assert_eq!(wire.model.as_deref(), Some("test-model"));
    assert_eq!(wire.past_attempts.len(), 1);
    let attempt = &wire.past_attempts[0];
    assert_eq!(attempt.id.as_deref(), Some("previous"));
    assert_eq!(attempt.title.as_deref(), Some("Previous review"));
    assert_eq!(attempt.updated_at, Some(123.5));
    assert_eq!(attempt.error_reason.as_deref(), Some("Failed review"));
}

#[test]
fn test_session詳細の転送_ドメインの状態と操作可否を既存の形式で配る() {
    use crate::domain::workflow::{NodeCompletionSignalState as S, NodeProcessPresence};
    use crate::domain::workspace_tree::{
        WorkspaceNodeKind, WorkspaceNodeStatus, WorkspaceNodeStatusClassification,
        WorkspaceTreeNode,
    };
    // Given
    let node = WorkspaceTreeNode {
        id: "node".into(),
        title: "Review".into(),
        parent_id: Some("execution".into()),
        sibling_order: 0,
        kind: WorkspaceNodeKind::WorkflowSession,
        status: WorkspaceNodeStatus::Waiting,
        process_presence: NodeProcessPresence::ConfirmedAbsent,
        status_classification: WorkspaceNodeStatusClassification::Attention,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: None,
        error_reason: Some("needs attention".into()),
        updated_at_bits: 124.0_f64.to_bits(),
        execution_id: Some("execution".into()),
        node_execution_id: Some("visit".into()),
        node_name: Some("review".into()),
        attempt: Some(2),
        execution_parent: None,
        retry_predecessor_id: None,
        past_attempt_ids: vec!["previous".into()],
        is_retry_history: false,
        completion_signals: S::Pending,
        has_artifact: true,
        session_id: Some("session".into()),
        model: Some("test-model".into()),
        provider: None,
        can_rename: true,
        can_approve: true,
        can_retry: false,
        can_resume_session: true,
        can_abort: false,
        can_archive: false,
        display_command: None,
        shell: None,
        cwd: None,
        command_result: None,
        worktree: None,
        dynamic_fanout: false,
    };
    for (signals, submit, stop, waiting) in [
        (S::Pending, false, false, None),
        (S::SubmitReceived, true, false, Some("stop")),
        (S::StopReceived, false, true, Some("submit")),
        (S::Ready, true, true, None),
    ] {
        let mut node = node.clone();
        node.completion_signals = signals;
        // When
        let message = wire::WorkspaceNodeDetailDto::try_from((node, true)).unwrap();
        let actual =
            wire::from_message("releash.client.v1.WorkspaceNodeDetailDto", &message).unwrap();
        // Then
        let mut expected = serde_json::json!({
            "id": "node", "title": "Review", "classification": "attention", "status": "waiting",
            "processPresence": "confirmed_absent", "nodeExecutionId": "visit", "nodeName": "review",
            "model": "test-model", "pastAttempts": [], "executionId": "execution", "canAbort": true,
            "submitReceived": submit, "stopReceived": stop,
            "hasArtifact": true, "errorReason": "needs attention", "updatedAt": 124.0,
            "capabilities": {"canRename": true, "canApprove": true, "canRetry": false, "canResumeSession": true},
            "content": {"kind": "session", "sessionId": "session"}
        });
        if let Some(waiting) = waiting {
            expected["waitingFor"] = waiting.into();
        }
        assert_eq!(actual, expected);
    }
}

mod workflow_definition {
    use super::*;
    use crate::domain::workflow as domain;
    use std::collections::BTreeMap;

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
                include_str!("../../gateway/workflow/fixtures/valid/predicate-routing.yml")
                    .replace("{and: [passed, {or: [clean, skipped]}]}", &on.to_string());
            let definition: domain::WorkflowDefinitionContent =
                serde_saphyr::from_str(&source).unwrap();
            // When
            let value = workflow_json(&definition, domain::WorkflowSourceFormat::Yaml);
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
            let value = wire::from_message(
                "releash.client.v1.RuleDto",
                &wire::RuleDto::try_from(rule).unwrap(),
            )
            .unwrap();
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
            let dto = wire::NodeDefinitionDto::try_from(node).unwrap();
            let value = wire::from_message("releash.client.v1.NodeDefinitionDto", &dto).unwrap();
            // Then
            assert_eq!(value.get("completion"), expected.as_ref());
            assert_eq!(
                wire::from_message(
                    "releash.client.v1.NodeDefinitionDto",
                    &wire::to_message::<wire::NodeDefinitionDto>(
                        "releash.client.v1.NodeDefinitionDto",
                        value.clone()
                    )
                    .unwrap()
                )
                .unwrap(),
                value
            );
        }
    }

    #[test]
    fn test_delegate表示dto_配線と述語と上限を含むcompletionを保持する() {
        // Given
        let workflow: domain::WorkflowDefinitionContent = serde_saphyr::from_str("name: test\ndescription: test\nnodes:\n  main: {session: {provider: codex}, artifact: result, completion: {require: approval, delegate: {child: check, inputs: {task: main.task, zebra: main, apple: request}, when: child.ok, max_iterations: 2}}}\n  check: {command: check}").unwrap();
        // When
        let dto = wire::NodeDefinitionDto::try_from(workflow.node_by_name("main").unwrap().clone())
            .unwrap();
        let value = wire::from_message("releash.client.v1.NodeDefinitionDto", &dto).unwrap();
        // Then
        assert_eq!(value["completion"]["require"], "approval");
        assert_eq!(value["completion"]["delegate"]["child"], "check");
        assert_eq!(value["completion"]["delegate"]["when"], "child.ok");
        assert_eq!(value["completion"]["delegate"]["max_iterations"], 2);
        assert_eq!(
            value["completion"]["delegate"]["inputs"],
            serde_json::json!([
                {"parameter": "task", "source": "main.task"}, {"parameter": "zebra", "source": "main"}, {"parameter": "apple", "source": "request"}
            ])
        );
        assert_eq!(
            wire::from_message(
                "releash.client.v1.NodeDefinitionDto",
                &wire::to_message::<wire::NodeDefinitionDto>(
                    "releash.client.v1.NodeDefinitionDto",
                    value.clone()
                )
                .unwrap()
            )
            .unwrap(),
            value
        );
    }

    #[test]
    fn test_workflow出力_既存の転送形式で直列化する() {
        // Given
        let workflow: domain::WorkflowDefinitionContent = serde_saphyr::from_str(
            r#"
name: wf
description: desc
schemas:
  plan: {type: object, properties: {passed: {type: boolean}}, required: [passed]}
nodes:
  node:
    session: {provider: claude, facets: {instruction: inst}}
    artifact: plan
    input: [{item: plan}]
"#,
        )
        .unwrap();
        // When
        let actual = workflow_json(&workflow, domain::WorkflowSourceFormat::Yaml);

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
                        "properties": {"passed": {"type": "boolean"}},
                        "required": ["passed"]
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
        let workflow = domain::WorkflowDefinitionContent {
            name: "lua-workflow".to_string(),
            description: "Lua".to_string(),
            ..domain::WorkflowDefinitionContent::default()
        };

        let value = workflow_json(&workflow, domain::WorkflowSourceFormat::Lua);

        assert_eq!(value["sourceFormat"], "lua");
        assert!(serde_json::to_value(workflow)
            .unwrap()
            .get("sourceFormat")
            .is_none());
    }

    #[test]
    fn workflow_to_dto_maps_knowledge_refs_to_ordered_json_array() {
        let definition = domain::WorkflowDefinitionContent {
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

        let value = workflow_json(&definition, domain::WorkflowSourceFormat::Yaml);

        assert_eq!(
            value["nodes"][0]["session"]["facets"]["knowledge"],
            serde_json::json!(["knowledge-a", "knowledge-b"])
        );
        assert_eq!(
            workflow_json(&definition, domain::WorkflowSourceFormat::Yaml)["nodes"][0]["session"]
                ["provider"],
            serde_json::json!("codex")
        );
        assert_eq!(
            workflow_json(&definition, domain::WorkflowSourceFormat::Yaml)["nodes"][0]["session"]
                ["permission"],
            serde_json::json!("read-only")
        );
    }

    #[test]
    fn workflow_to_dto_preserves_loop_guard() {
        let definition = domain::WorkflowDefinitionContent {
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

        let value = workflow_json(&definition, domain::WorkflowSourceFormat::Yaml);

        assert_eq!(
            value["nodes"][0]["sequence"]["children"][0]["rules"][0],
            serde_json::json!({
                "type": "loop_guard",
                "max_iterations": 2,
                "on_exhausted": "done"
            })
        );
    }

    #[test]
    fn fanout_spec_dto_serializes_child_and_items_sources() {
        let literal = domain::FanoutSpec {
            children: vec![domain::ChildEntry {
                name: "review".into(),
                inputs: vec![],
                rules: None,
            }],
            items: Some(domain::ItemsSource::Literal(vec![
                serde_json::json!({"thread_id":"thread-1"}),
            ])),
        };
        assert_eq!(
            wire::from_message(
                "releash.client.v1.FanoutSpecDto",
                &wire::FanoutSpecDto::try_from(literal).unwrap()
            )
            .unwrap(),
            serde_json::json!({
                "children": [{"name": "review"}],
                "items": [{"thread_id": "thread-1"}]
            })
        );

        let reference = domain::FanoutSpec {
            children: ["review-opus", "review-gpt"]
                .into_iter()
                .map(|name| domain::ChildEntry {
                    name: name.into(),
                    inputs: vec![],
                    rules: None,
                })
                .collect(),
            items: Some(domain::ItemsSource::ArtifactField {
                node: "scan".into(),
                field_path: domain::value_objects::FieldPath::new(["threads"]),
            }),
        };
        assert_eq!(
            wire::from_message(
                "releash.client.v1.FanoutSpecDto",
                &wire::FanoutSpecDto::try_from(reference).unwrap()
            )
            .unwrap(),
            serde_json::json!({
                "children": [{"name": "review-opus"}, {"name": "review-gpt"}],
                "items": "scan.threads"
            })
        );
    }

    fn workflow_json(
        content: &domain::WorkflowDefinitionContent,
        format: domain::WorkflowSourceFormat,
    ) -> serde_json::Value {
        let definition = domain::WorkflowDefinition::restore(
            content.name.clone(),
            "/workflows".into(),
            domain::WorkflowSource::File {
                path: "/workflows/wf.yml".into(),
            },
            format,
            Ok(content.clone()),
        );
        wire::from_message(
            "releash.client.v1.WorkflowDto",
            &wire::WorkflowDto::try_from(definition).unwrap(),
        )
        .unwrap()
    }
}
