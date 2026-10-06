use super::*;
use crate::adaptor::gateway::workflow::schema::Rule;
use crate::adaptor::gateway::workflow::test_helpers::*;

const MERGED_REFERENCES: &str = include_str!("fixtures/valid/sequence-merged-references.yml");

const FANOUT_REFERENCES: &str = include_str!("fixtures/valid/fanout-map-references.yml");

const FANOUT_ROUTING: &str = include_str!("fixtures/valid/fanout-map-routing.yml");

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
fn test_fanout多段参照の診断_未解決の配線を_wfr007と絶対位置で報告する() {
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

#[test]
fn test_fanoutの判別規則の診断_終端の型とrequiredで_wft001と_wft002を返す() {
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
fn test_診断itemは省略されたoptional_fieldをnoneとしてdeserializeする() {
    // Given
    let value = serde_json::json!({
        "code": "X",
        "severity": "error",
        "stage": "parse_shape",
        "message": "m"
    });

    // When
    let item = serde_json::from_value::<
        crate::adaptor::presenter::workflow_api::DiagnosticItemResponse,
    >(value)
    .unwrap();

    // Then
    assert!(item.span.is_none());
    assert!(item.workflow_name.is_none());
    assert!(item.node_name.is_none());
    assert!(item.facet_key.is_none());
    assert!(item.facet_kind.is_none());
    assert!(item.field.is_none());
}

#[test]
fn test_診断spanは省略されたsourceをnoneとしてdeserializeする() {
    // Given
    let value = serde_json::json!({
        "start_line": 7,
        "start_col": 5,
        "end_line": 7,
        "end_col": 6
    });

    // When
    let span = serde_json::from_value::<
        crate::adaptor::presenter::workflow_api::DiagnosticSpanResponse,
    >(value)
    .unwrap();

    // Then
    assert!(span.source.is_none());
}

#[test]
fn workflow_source_diagnosticsは未知fieldとkeywordを拒否する() {
    let cases = [
        (
            "root",
            r#"
name: unknown-root-field
description: unknown root field
future_field: ignored
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
        ),
        (
            "node",
            r#"
name: unknown-node-field
description: unknown node field
nodes:
  main:
    future_field: ignored
    session:
      provider: claude
      facets:
        instruction: implement
"#,
        ),
        (
            "session",
            r#"
name: unknown-session-field
description: unknown session field
nodes:
  main:
    session:
      provider: claude
      future_field: ignored
      facets:
        instruction: implement
"#,
        ),
        (
            "session.facets",
            r#"
name: unknown-facet-field
description: unknown session facet field
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
        future_field: ignored
"#,
        ),
        (
            "fanout",
            r#"
name: unknown-fanout-field
description: unknown fanout field
nodes:
  main:
    fanout:
      children:
      - worker
      future_field: ignored
  worker:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
        ),
        (
            "rule",
            r#"
name: unknown-rule-field
description: unknown rule field
nodes:
  main:
    sequence:
      children:
      - work:
          rules:
          - next: review
            future_field: ignored
      - review
  work:
    session:
      provider: claude
      facets:
        instruction: implement
  review:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
        ),
        (
            "schemas",
            r#"
name: unknown-schema-keyword
description: unknown schema keyword
schemas:
  review:
    type: object
    future_keyword: ignored
    properties:
      verdict:
        type: boolean
    required:
      - verdict
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: implement
"#,
        ),
    ];

    for (label, source) in cases {
        let known = source
            .lines()
            .filter(|line| !line.contains("future_"))
            .collect::<Vec<_>>()
            .join("\n");
        let known_diagnosis = diagnose_workflow_source(&known, None);
        assert!(
            known_diagnosis.diagnostics.is_empty(),
            "{label} without unknown input must be accepted: {:?}",
            known_diagnosis.diagnostics
        );

        let diagnosis = diagnose_workflow_source(source, None);
        assert!(
            diagnosis
                .diagnostics
                .iter()
                .any(|item| item.code == "WFS002"),
            "unknown input at {label} must be rejected: {:?}",
            diagnosis.diagnostics
        );
        assert!(diagnosis.workflow.is_none(), "{label}");
    }
}

#[test]
fn validation_error_code_stage_uses_typed_variants() {
    let cases = vec![
        (
            validation::ValidationError::InvalidSchema {
                schema: "list".to_string(),
                kind: InvalidSchemaKind::UnknownSchemaReference,
                reason: "renamed wording".to_string(),
            },
            "WFR002",
            DiagnosticStage::Resolve,
        ),
        (
            validation::ValidationError::InvalidSchema {
                schema: "bad".to_string(),
                kind: InvalidSchemaKind::InvalidDeclaration,
                reason: "renamed wording".to_string(),
            },
            "WFS002",
            DiagnosticStage::ParseShape,
        ),
        (
            validation::ValidationError::InvalidArtifactReference {
                reference: "request".to_string(),
                kind: InvalidArtifactReferenceKind::ReservedArtifactName,
                reason: "renamed wording".to_string(),
            },
            "WFR004",
            DiagnosticStage::Resolve,
        ),
        (
            validation::ValidationError::InvalidArtifactReference {
                reference: "item".to_string(),
                kind: InvalidArtifactReferenceKind::UnknownParameter,
                reason: "renamed wording".to_string(),
            },
            "WFR003",
            DiagnosticStage::Resolve,
        ),
        (
            validation::ValidationError::CompositeInclusionCycle {
                node: "part".to_string(),
                cycle: "part -> part".to_string(),
            },
            "WFC008",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidInputWiring(Box::new(
                validation::InputWiringViolation {
                    node: "main".to_string(),
                    child: "consume".to_string(),
                    parameter: "spec".to_string(),
                    source: "ghost".to_string(),
                    kind: validation::InputWiringKind::UnknownSource,
                    reason: "renamed wording".to_string(),
                },
            )),
            "WFR007",
            DiagnosticStage::Resolve,
        ),
        (
            validation::ValidationError::InvalidArtifactReference {
                reference: "plan.field".to_string(),
                kind: InvalidArtifactReferenceKind::UnknownField,
                reason: "renamed wording".to_string(),
            },
            "WFR003",
            DiagnosticStage::Resolve,
        ),
        (
            validation::ValidationError::InvalidArtifactReference {
                reference: "bad ref".to_string(),
                kind: InvalidArtifactReferenceKind::InvalidInputRef,
                reason: "renamed wording".to_string(),
            },
            "WFR003",
            DiagnosticStage::Resolve,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::WhenFieldNotBoolean,
                reason: "renamed wording".to_string(),
            },
            "WFT001",
            DiagnosticStage::Typecheck,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::SwitchFieldNotEnum,
                reason: "renamed wording".to_string(),
            },
            "WFT002",
            DiagnosticStage::Typecheck,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::SwitchUnknownCase,
                reason: "renamed wording".to_string(),
            },
            "WFT002",
            DiagnosticStage::Typecheck,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::DiscriminatorWithoutArtifact,
                reason: "renamed wording".to_string(),
            },
            "WFT006",
            DiagnosticStage::Typecheck,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::SwitchMissingCases,
                reason: "renamed wording".to_string(),
            },
            "WFC004",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::CycleWithoutLoopGuard,
                reason: "renamed wording".to_string(),
            },
            "WFC005",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::LoopGuardMaxIterations,
                reason: "renamed wording".to_string(),
            },
            "WFC005",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::SwitchRequiresNext,
                reason: "renamed wording".to_string(),
            },
            "WFC003",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::SwitchExhaustiveHasNext,
                reason: "renamed wording".to_string(),
            },
            "WFC003",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::MultipleNextCatchAll,
                reason: "renamed wording".to_string(),
            },
            "WFC003",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::MultipleDiscriminators,
                reason: "renamed wording".to_string(),
            },
            "WFC002",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::MultipleLoopGuards,
                reason: "renamed wording".to_string(),
            },
            "WFC002",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::InvalidRules {
                node: "route".to_string(),
                kind: InvalidRuleKind::StandaloneNextWithDiscriminator,
                reason: "renamed wording".to_string(),
            },
            "WFC002",
            DiagnosticStage::ControlFlow,
        ),
        (
            validation::ValidationError::UnreachableNode {
                node: "orphan".to_string(),
            },
            "WFC001",
            DiagnosticStage::ControlFlow,
        ),
    ];

    for (error, expected_code, expected_stage) in cases {
        let (code, stage) = validation_error_code_stage(&error);
        assert_eq!(code, expected_code, "wrong code for {error:?}");
        assert_eq!(stage, expected_stage, "wrong stage for {error:?}");
    }
}

#[test]
fn deserialize_error_diagnostic_classifies_error_messages() {
    let span_map = YamlSpanMap::parse("name: sample\nnodes: {}\n").unwrap();
    let cases = [
        ("unknown field `future_field`", "WFS002"),
        ("unknown variant `future_variant`", "WFS002"),
        ("when rule requires sibling next", "WFS003"),
        ("YAML syntax problem", "WFS001"),
        ("unclassified deserialize problem", "WFS002"),
    ];

    for (message, expected_code) in cases {
        let error = <serde_saphyr::Error as serde::de::Error>::custom(message);
        let item = deserialize_error_diagnostic(&error, &span_map, Some("sample"));
        assert_eq!(item.code, expected_code, "wrong code for {message}");
        assert_eq!(item.stage, DiagnosticStage::ParseShape);
    }
}

#[test]
fn empty_workflow_nodes_diagnostic_uses_node_vocabulary_and_targets_nodes_field() {
    assert_eq!(
        validation_error_context(&validation::ValidationError::EmptyNodes),
        (None, Some("nodes".to_string()))
    );
}

#[test]
fn test_診断_nodesマップの重複キーはwfs006で再出現位置を指す() {
    let source = r#"name: dup-node
description: duplicate node key
nodes:
  main:
    command: printf first
  main:
    command: printf second
"#;

    let diagnosis = diagnose_workflow_source(source, Some("dup-node"));
    let item = diagnosis
        .diagnostics
        .iter()
        .find(|item| item.code == "WFS006")
        .expect("duplicate node key must produce WFS006");
    assert_eq!(item.severity, Severity::Error);
    assert_eq!(item.stage, DiagnosticStage::ParseShape);
    assert_eq!(item.workflow_name.as_deref(), Some("dup-node"));
    let span = item
        .span
        .as_ref()
        .expect("duplicate node key diagnostic must carry a span");
    assert_eq!(
        span.start_line, 6,
        "span must point at the re-occurring node key"
    );
    assert!(diagnosis.workflow.is_none());
}

#[test]
fn test_診断_main不在はwfr006になる() {
    let source = r#"name: no-main
description: nodes without the main root node
nodes:
  prepare:
    command: printf prepare
"#;

    let diagnosis = diagnose_workflow_source(source, None);
    let item = diagnosis
        .diagnostics
        .iter()
        .find(|item| item.code == "WFR006")
        .expect("nodes without main must produce WFR006");
    assert_eq!(item.severity, Severity::Error);
    assert_eq!(item.stage, DiagnosticStage::Resolve);
    assert_eq!(item.field.as_deref(), Some("nodes"));
    assert!(item.message.contains("main"));
}

#[test]
fn test_診断_予約語node名はwfr004になる() {
    let source = r#"name: reserved-node-name
description: sequence is a reserved node name
nodes:
  main:
    command: printf main
    rules:
      - next: sequence
  sequence:
    command: printf work
"#;

    let diagnosis = diagnose_workflow_source(source, None);
    let item = diagnosis
        .diagnostics
        .iter()
        .find(|item| item.code == "WFR004")
        .expect("reserved node name must produce WFR004");
    assert_eq!(item.severity, Severity::Error);
    assert_eq!(item.stage, DiagnosticStage::Resolve);
    assert_eq!(item.node_name.as_deref(), Some("sequence"));
    assert!(item.span.is_some());
    assert!(diagnosis.workflow.is_none());
}

mod delegate_diagnostics_tests {
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
            assert!(
                crate::adaptor::gateway::workflow::storage::parse_workflow_source(
                    &value.to_string(),
                    std::path::Path::new(".")
                )
                .is_err()
            );
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
            assert!(
                crate::adaptor::gateway::workflow::storage::parse_workflow_source(
                    &value.to_string(),
                    std::path::Path::new(".")
                )
                .is_err()
            );
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
                assert!(
                    crate::adaptor::gateway::workflow::storage::parse_workflow_source(
                        &value.to_string(),
                        std::path::Path::new(".")
                    )
                    .is_err()
                );
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
        value["schemas"]["composed"]["properties"]["child"]["properties"]["passed"] =
            json!("string");
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
        value["nodes"]["main"]["completion"]["delegate"]["inputs"] =
            json!({"result": "main.child"});
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
                "zero" => {
                    value["nodes"]["main"]["completion"]["delegate"]["max_iterations"] = json!(0)
                }
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
                    value["nodes"]["verify"] = json!({"session": {"provider": "codex", "facets": {"instruction": "check"}}})
                }
                _ => unreachable!(),
            }
            // When
            let diagnosis = check(&value);
            // Then
            assert!(
                diagnosis.diagnostics.iter().any(|d| d.code == code
                    && d.stage == stage
                    && d.field.as_deref() == Some(field)),
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
        let span =
            crate::adaptor::gateway::workflow::span_map::YamlSpanMap::parse(&source).unwrap();
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
}

mod restored_memory_cases {
    use super::super::*;

    #[test]
    pub fn test_fanout変更後のbuiltin定義_8本すべて診断ゼロでloadする() {
        // Given
        let summaries = crate::adaptor::gateway::workflow::builtin::list_builtin_workflows();
        for summary in summaries {
            let source =
                crate::adaptor::gateway::workflow::builtin::builtin_workflow_source(&summary.name)
                    .unwrap();

            // When
            let diagnosis = diagnose_workflow_source(source, Some(&summary.name));
            let loaded = diagnosis.workflow;

            // Then
            assert!(
                diagnosis.diagnostics.is_empty(),
                "{}: {:?}",
                summary.name,
                diagnosis.diagnostics
            );
            assert!(loaded.is_some(), "{loaded:?}");
        }
    }
}
