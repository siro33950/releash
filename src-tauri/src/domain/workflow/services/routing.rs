use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

use crate::domain::workflow::services::contract_schema::{self, RoutingFieldKind};
use crate::domain::workflow::value_objects::{
    NodeDefinition, NodeKind, Rule, SchemaDef, WorkflowDefinition,
};
use crate::domain::workflow::WorkflowError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    Completed,
    TransitionTo(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingValidationError {
    UnknownRuleTarget { step: String, target: String },
    Invalid { step: String, reason: String },
}

pub fn validate_rules(workflow: &WorkflowDefinition) -> Vec<RoutingValidationError> {
    let node_names: BTreeSet<_> = workflow
        .nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    let mut errors = Vec::new();

    for node in &workflow.nodes {
        errors.extend(validate_node_rules(workflow, node, &node_names));
    }
    errors.extend(validate_cycles_have_loop_guard(workflow));

    errors
}

pub fn route(
    workflow: &WorkflowDefinition,
    current_index: usize,
    artifact: Option<&Value>,
    node_execution_counts: &HashMap<String, u32>,
) -> Result<RouteDecision, WorkflowError> {
    let node = workflow.nodes.get(current_index).ok_or_else(|| {
        WorkflowError::validation(format!("node index out of range: {current_index}"))
    })?;

    let Some(target) = raw_target(node, artifact)? else {
        return Ok(RouteDecision::Completed);
    };
    guarded_target(workflow, target, node_execution_counts)
}

pub fn rule_targets(rule: &Rule) -> Vec<&str> {
    match rule {
        Rule::When { then, next, .. } => vec![then.as_str(), next.as_str()],
        Rule::Switch { cases, next, .. } => cases
            .values()
            .map(String::as_str)
            .chain(next.iter().map(String::as_str))
            .collect(),
        Rule::LoopGuard { on_exhausted, .. } => vec![on_exhausted.as_str()],
        Rule::Next(next) => vec![next.as_str()],
    }
}

pub fn loop_guard(node: &NodeDefinition) -> Option<(u32, &str)> {
    node.rules.iter().find_map(|rule| match rule {
        Rule::LoopGuard {
            max_iterations,
            on_exhausted,
        } => Some((*max_iterations, on_exhausted.as_str())),
        _ => None,
    })
}

fn validate_node_rules(
    workflow: &WorkflowDefinition,
    node: &NodeDefinition,
    node_names: &BTreeSet<&str>,
) -> Vec<RoutingValidationError> {
    let mut errors = Vec::new();
    let mut discriminator_count = 0usize;
    let mut loop_guard_count = 0usize;
    let mut next_count = 0usize;

    for rule in &node.rules {
        for target in rule_targets(rule) {
            if !node_names.contains(target) {
                errors.push(RoutingValidationError::UnknownRuleTarget {
                    step: node.name.clone(),
                    target: target.to_string(),
                });
            }
        }
        match rule {
            Rule::When { .. } | Rule::Switch { .. } => discriminator_count += 1,
            Rule::LoopGuard { max_iterations, .. } => {
                loop_guard_count += 1;
                if *max_iterations == 0 {
                    errors.push(invalid(
                        node,
                        "loop_guard.max_iterations must be greater than 0",
                    ));
                }
            }
            Rule::Next(_) => next_count += 1,
        }
    }

    if discriminator_count > 1 {
        errors.push(invalid(
            node,
            "rules can contain at most one when or switch discriminator",
        ));
    }
    if loop_guard_count > 1 {
        errors.push(invalid(node, "rules can contain at most one loop_guard"));
    }
    if next_count > 1 {
        errors.push(invalid(
            node,
            "rules can contain at most one next catch-all",
        ));
    }
    if discriminator_count > 0 && next_count > 0 {
        errors.push(invalid(
            node,
            "standalone next cannot be combined with when or switch",
        ));
    }

    let discriminator = node
        .rules
        .iter()
        .find(|rule| matches!(rule, Rule::When { .. } | Rule::Switch { .. }));
    match discriminator {
        Some(Rule::When { on, .. }) => {
            if let Err(reason) =
                validate_routing_field(workflow, node, on, RoutingFieldKind::Boolean)
            {
                errors.push(invalid(node, reason));
            }
        }
        Some(Rule::Switch { on, cases, next }) => {
            match validate_routing_field(workflow, node, on, RoutingFieldKind::Enum) {
                Ok(enum_values) => {
                    let enum_set: BTreeSet<_> = enum_values.iter().map(String::as_str).collect();
                    let case_set: BTreeSet<_> = cases.keys().map(String::as_str).collect();
                    for case in case_set.difference(&enum_set) {
                        errors.push(invalid(
                            node,
                            format!("switch case '{case}' is not declared in enum field '{on}'"),
                        ));
                    }
                    let missing: Vec<_> = enum_set.difference(&case_set).copied().collect();
                    let needs_p11_next = node.is_command() && node.artifact.is_some() && on != "ok";
                    if missing.is_empty() {
                        if next.is_some() && !needs_p11_next {
                            errors.push(invalid(
                                node,
                                "exhaustive switch cannot also define next catch-all",
                            ));
                        }
                        if needs_p11_next && next.is_none() {
                            errors.push(invalid(
                                node,
                                "command artifact routing on Contract field requires next catch-all",
                            ));
                        }
                    } else if next.is_none() {
                        errors.push(invalid(
                            node,
                            format!(
                                "switch on '{on}' is missing enum cases [{}] and requires next",
                                missing.join(", ")
                            ),
                        ));
                    }
                }
                Err(reason) => errors.push(invalid(node, reason)),
            }
        }
        _ => {}
    }

    if discriminator.is_some() && matches!(node.kind, NodeKind::Fanout(_)) {
        errors.push(invalid(
            node,
            "fanout nodes cannot use when or switch rules",
        ));
    }
    if discriminator.is_some() && node.artifact.is_none() && !node.is_command() {
        errors.push(invalid(
            node,
            "nodes without an artifact cannot use when or switch rules",
        ));
    }

    errors
}

fn validate_routing_field(
    workflow: &WorkflowDefinition,
    node: &NodeDefinition,
    field: &str,
    expected: RoutingFieldKind,
) -> Result<Vec<String>, String> {
    if node.is_command() && field == "ok" {
        if expected == RoutingFieldKind::Boolean {
            return Ok(Vec::new());
        }
        return Err("switch.on cannot reference command reserved boolean field 'ok'".to_string());
    }

    let contract_name = node.artifact.as_deref().ok_or_else(|| {
        format!("routing field '{field}' requires an artifact Contract on this node")
    })?;
    let schema = workflow
        .schemas
        .get(contract_name)
        .ok_or_else(|| format!("artifact Contract '{contract_name}' is not declared in schemas"))?;
    let kind = contract_schema::routing_field_kind(schema, field).map_err(|err| match err {
        contract_schema::RoutingFieldError::NotObject => {
            format!("artifact Contract '{contract_name}' is not an object")
        }
        contract_schema::RoutingFieldError::MissingProperty { .. } => {
            format!("routing field '{field}' is not declared on Contract '{contract_name}'")
        }
        contract_schema::RoutingFieldError::NotRequired { .. } => {
            format!("routing field '{field}' must be required on Contract '{contract_name}'")
        }
        contract_schema::RoutingFieldError::NotBooleanOrEnum { .. } => {
            format!("routing field '{field}' must be boolean or string enum")
        }
    })?;
    if kind != expected {
        return Err(match expected {
            RoutingFieldKind::Boolean => {
                format!("when.on field '{field}' must be a required boolean")
            }
            RoutingFieldKind::Enum => {
                format!("switch.on field '{field}' must be a required enum")
            }
        });
    }
    Ok(enum_values(schema, field).unwrap_or_default())
}

fn enum_values(schema: &SchemaDef, field: &str) -> Option<Vec<String>> {
    let SchemaDef::Object { properties, .. } = schema else {
        return None;
    };
    let SchemaDef::String {
        r#enum: Some(values),
    } = properties.get(field)?
    else {
        return None;
    };
    Some(values.clone())
}

fn raw_target(
    node: &NodeDefinition,
    artifact: Option<&Value>,
) -> Result<Option<String>, WorkflowError> {
    let discriminator = node
        .rules
        .iter()
        .find(|rule| matches!(rule, Rule::When { .. } | Rule::Switch { .. }));
    match discriminator {
        Some(Rule::When { on, then, next }) => {
            let is_true = artifact
                .and_then(|value| value.get(on))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(Some(if is_true { then } else { next }.clone()))
        }
        Some(Rule::Switch { on, cases, next }) => {
            if let Some(target) = artifact
                .and_then(|value| value.get(on))
                .and_then(Value::as_str)
                .and_then(|value| cases.get(value))
            {
                return Ok(Some(target.clone()));
            }
            next.clone().map(Some).ok_or_else(|| {
                WorkflowError::validation(format!(
                    "No matching switch case for node '{}' and no next catch-all",
                    node.name
                ))
            })
        }
        _ => Ok(node.rules.iter().find_map(|rule| match rule {
            Rule::Next(next) => Some(next.clone()),
            _ => None,
        })),
    }
}

pub fn guarded_target(
    workflow: &WorkflowDefinition,
    mut target: String,
    node_execution_counts: &HashMap<String, u32>,
) -> Result<RouteDecision, WorkflowError> {
    for _ in 0..workflow.nodes.len() {
        let target_node = workflow
            .nodes
            .iter()
            .find(|node| node.name == target)
            .ok_or_else(|| WorkflowError::validation(format!("node not found: {target}")))?;
        let Some((max_iterations, on_exhausted)) = loop_guard(target_node) else {
            return Ok(RouteDecision::TransitionTo(target));
        };
        let count = node_execution_counts.get(&target).copied().unwrap_or(0);
        if count < max_iterations {
            return Ok(RouteDecision::TransitionTo(target));
        }
        target = on_exhausted.to_string();
    }
    Err(WorkflowError::validation(
        "loop_guard on_exhausted chain depth exceeded",
    ))
}

fn validate_cycles_have_loop_guard(workflow: &WorkflowDefinition) -> Vec<RoutingValidationError> {
    let node_names: BTreeSet<_> = workflow
        .nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for node in &workflow.nodes {
        graph.entry(node.name.as_str()).or_default();
        for target in node.rules.iter().flat_map(rule_targets) {
            if node_names.contains(target) {
                graph.entry(node.name.as_str()).or_default().insert(target);
            }
        }
    }

    let mut errors = Vec::new();
    let node_by_name: BTreeMap<_, _> = workflow
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    for component in cyclic_strongly_connected_components(&node_names, &graph) {
        let has_guard_on_cycle = component
            .iter()
            .filter_map(|name| node_by_name.get(*name).copied())
            .any(|node| loop_guard(node).is_some());
        if !has_guard_on_cycle {
            for name in component {
                if let Some(node) = node_by_name.get(name).copied() {
                    errors.push(invalid(
                        node,
                        "cycle reachable from this node has no loop_guard on cycle nodes",
                    ));
                }
            }
        }
    }
    errors
}

fn cyclic_strongly_connected_components<'a>(
    node_names: &BTreeSet<&'a str>,
    graph: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> Vec<BTreeSet<&'a str>> {
    let mut components = Vec::new();
    let mut seen = BTreeSet::new();

    for start in node_names {
        let component: BTreeSet<_> = node_names
            .iter()
            .copied()
            .filter(|candidate| {
                is_reachable(start, candidate, graph, &mut HashSet::new())
                    && is_reachable(candidate, start, graph, &mut HashSet::new())
            })
            .collect();
        if component.is_empty() || !is_cyclic_component(&component, graph) {
            continue;
        }
        let key: Vec<_> = component.iter().copied().collect();
        if seen.insert(key) {
            components.push(component);
        }
    }

    components
}

fn is_cyclic_component<'a>(
    component: &BTreeSet<&'a str>,
    graph: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> bool {
    if component.len() > 1 {
        return true;
    }
    let Some(node) = component.iter().next().copied() else {
        return false;
    };
    graph
        .get(node)
        .is_some_and(|targets| targets.contains(node))
}

fn is_reachable<'a>(
    current: &'a str,
    target: &'a str,
    graph: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    visited: &mut HashSet<&'a str>,
) -> bool {
    if current == target {
        return true;
    }
    if !visited.insert(current) {
        return false;
    }
    if let Some(targets) = graph.get(current) {
        for next in targets {
            if is_reachable(next, target, graph, visited) {
                return true;
            }
        }
    }
    false
}

fn invalid(node: &NodeDefinition, reason: impl Into<String>) -> RoutingValidationError {
    RoutingValidationError::Invalid {
        step: node.name.clone(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{CommandSpec, FacetRefs, NodeKind, SessionSpec};
    use proptest::prelude::*;

    fn bool_schema(field: &str) -> SchemaDef {
        SchemaDef::Object {
            properties: BTreeMap::from([(field.to_string(), SchemaDef::Boolean)]),
            required: BTreeSet::from([field.to_string()]),
            additional_properties: true,
        }
    }

    fn enum_schema(field: &str, values: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties: BTreeMap::from([(
                field.to_string(),
                SchemaDef::String {
                    r#enum: Some(values.iter().map(|value| (*value).to_string()).collect()),
                },
            )]),
            required: BTreeSet::from([field.to_string()]),
            additional_properties: true,
        }
    }

    fn mixed_schema(fields: Vec<(&str, SchemaDef)>, required: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties: fields
                .into_iter()
                .map(|(name, schema)| (name.to_string(), schema))
                .collect(),
            required: required.iter().map(|value| (*value).to_string()).collect(),
            additional_properties: true,
        }
    }

    fn session_node(name: &str, artifact: Option<&str>, rules: Vec<Rule>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(SessionSpec {
                facets: FacetRefs {
                    instruction: Some("inst".to_string()),
                    ..Default::default()
                },
                permission: Some("ask".to_string()),
                ..Default::default()
            }),
            artifact: artifact.map(ToOwned::to_owned),
            rules,
            ..Default::default()
        }
    }

    fn command_node(name: &str, artifact: Option<&str>, rules: Vec<Rule>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: "true".to_string(),
            }),
            artifact: artifact.map(ToOwned::to_owned),
            rules,
            ..Default::default()
        }
    }

    fn workflow(
        nodes: Vec<NodeDefinition>,
        schemas: BTreeMap<String, SchemaDef>,
    ) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas,
            nodes,
        }
    }

    fn assert_invalid_reason(wf: &WorkflowDefinition, expected: &str) {
        let errors = validate_rules(wf);
        assert!(
            errors.iter().any(|error| matches!(
                error,
                RoutingValidationError::Invalid { reason, .. } if reason.contains(expected)
            )),
            "expected invalid reason containing '{expected}', got {errors:?}"
        );
    }

    #[test]
    fn test_rules_empty_nodeはcompletedを返す() {
        let wf = workflow(vec![session_node("done", None, vec![])], BTreeMap::new());

        assert_eq!(
            route(&wf, 0, None, &HashMap::new()).unwrap(),
            RouteDecision::Completed
        );
    }

    #[test]
    fn test_rules_whenはboolean_fieldで一意に遷移する() {
        let wf = workflow(
            vec![
                session_node(
                    "judge",
                    Some("verdict"),
                    vec![Rule::When {
                        on: "passed".to_string(),
                        then: "done".to_string(),
                        next: "fix".to_string(),
                    }],
                ),
                session_node("done", None, vec![]),
                session_node("fix", None, vec![]),
            ],
            BTreeMap::from([("verdict".to_string(), bool_schema("passed"))]),
        );
        assert!(validate_rules(&wf).is_empty());
        assert_eq!(
            route(
                &wf,
                0,
                Some(&serde_json::json!({"passed": true})),
                &HashMap::new()
            )
            .unwrap(),
            RouteDecision::TransitionTo("done".to_string())
        );
        assert_eq!(
            route(
                &wf,
                0,
                Some(&serde_json::json!({"passed": false})),
                &HashMap::new()
            )
            .unwrap(),
            RouteDecision::TransitionTo("fix".to_string())
        );
    }

    #[test]
    fn test_rules_switchはenum網羅ならcatch_all不要() {
        let wf = workflow(
            vec![
                session_node(
                    "judge",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "decision".to_string(),
                        cases: BTreeMap::from([
                            ("SHIP".to_string(), "done".to_string()),
                            ("HOLD".to_string(), "fix".to_string()),
                        ]),
                        next: None,
                    }],
                ),
                session_node("done", None, vec![]),
                session_node("fix", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                enum_schema("decision", &["SHIP", "HOLD"]),
            )]),
        );
        assert!(validate_rules(&wf).is_empty());
    }

    #[test]
    fn test_rules_switch_enum抜けはnext必須() {
        let wf = workflow(
            vec![
                session_node(
                    "judge",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "decision".to_string(),
                        cases: BTreeMap::from([("SHIP".to_string(), "done".to_string())]),
                        next: None,
                    }],
                ),
                session_node("done", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                enum_schema("decision", &["SHIP", "HOLD"]),
            )]),
        );
        assert_eq!(validate_rules(&wf).len(), 1);
    }

    #[test]
    fn test_rules_command_contract_field参照はnext必須() {
        let wf = workflow(
            vec![
                command_node(
                    "judge",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "decision".to_string(),
                        cases: BTreeMap::from([
                            ("SHIP".to_string(), "done".to_string()),
                            ("HOLD".to_string(), "fix".to_string()),
                        ]),
                        next: None,
                    }],
                ),
                session_node("done", None, vec![]),
                session_node("fix", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                enum_schema("decision", &["SHIP", "HOLD"]),
            )]),
        );
        assert_eq!(validate_rules(&wf).len(), 1);
    }

    #[test]
    fn test_rules_validate_rulesは排他違反を拒否する() {
        let wf = workflow(
            vec![
                session_node(
                    "route",
                    Some("verdict"),
                    vec![
                        Rule::When {
                            on: "passed".to_string(),
                            then: "done".to_string(),
                            next: "fix".to_string(),
                        },
                        Rule::Switch {
                            on: "decision".to_string(),
                            cases: BTreeMap::from([("SHIP".to_string(), "done".to_string())]),
                            next: Some("fix".to_string()),
                        },
                    ],
                ),
                session_node("done", None, vec![]),
                session_node("fix", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                mixed_schema(
                    vec![
                        ("passed", SchemaDef::Boolean),
                        (
                            "decision",
                            SchemaDef::String {
                                r#enum: Some(vec!["SHIP".to_string(), "HOLD".to_string()]),
                            },
                        ),
                    ],
                    &["passed", "decision"],
                ),
            )]),
        );

        assert_invalid_reason(&wf, "at most one when or switch discriminator");
    }

    #[test]
    fn test_rules_validate_rulesは網羅済みswitchのnextを拒否する() {
        let wf = workflow(
            vec![
                session_node(
                    "route",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "decision".to_string(),
                        cases: BTreeMap::from([
                            ("SHIP".to_string(), "done".to_string()),
                            ("HOLD".to_string(), "fix".to_string()),
                        ]),
                        next: Some("fix".to_string()),
                    }],
                ),
                session_node("done", None, vec![]),
                session_node("fix", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                enum_schema("decision", &["SHIP", "HOLD"]),
            )]),
        );

        assert_invalid_reason(&wf, "exhaustive switch cannot also define next catch-all");
    }

    #[test]
    fn test_rules_validate_rulesは判別ruleをfanoutやartifact無しnodeで拒否する() {
        let fanout = NodeDefinition {
            name: "fanout".to_string(),
            kind: NodeKind::Fanout(Default::default()),
            artifact: Some("verdict".to_string()),
            rules: vec![Rule::When {
                on: "passed".to_string(),
                then: "done".to_string(),
                next: "done".to_string(),
            }],
            ..Default::default()
        };
        let fanout_wf = workflow(
            vec![fanout, session_node("done", None, vec![])],
            BTreeMap::from([("verdict".to_string(), bool_schema("passed"))]),
        );
        assert_invalid_reason(&fanout_wf, "fanout nodes cannot use when or switch rules");

        let no_artifact_wf = workflow(
            vec![
                session_node(
                    "route",
                    None,
                    vec![Rule::When {
                        on: "passed".to_string(),
                        then: "done".to_string(),
                        next: "done".to_string(),
                    }],
                ),
                session_node("done", None, vec![]),
            ],
            BTreeMap::new(),
        );
        assert_invalid_reason(
            &no_artifact_wf,
            "nodes without an artifact cannot use when or switch rules",
        );
    }

    #[test]
    fn test_rules_validate_rulesはwhen_switchの型不一致を拒否する() {
        let when_on_enum = workflow(
            vec![
                session_node(
                    "route",
                    Some("verdict"),
                    vec![Rule::When {
                        on: "decision".to_string(),
                        then: "done".to_string(),
                        next: "done".to_string(),
                    }],
                ),
                session_node("done", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                enum_schema("decision", &["SHIP", "HOLD"]),
            )]),
        );
        assert_invalid_reason(
            &when_on_enum,
            "when.on field 'decision' must be a required boolean",
        );

        let switch_on_bool = workflow(
            vec![
                session_node(
                    "route",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "passed".to_string(),
                        cases: BTreeMap::from([("true".to_string(), "done".to_string())]),
                        next: Some("done".to_string()),
                    }],
                ),
                session_node("done", None, vec![]),
            ],
            BTreeMap::from([("verdict".to_string(), bool_schema("passed"))]),
        );
        assert_invalid_reason(
            &switch_on_bool,
            "switch.on field 'passed' must be a required enum",
        );
    }

    #[test]
    fn test_rules_switch_field不在時はnextへfallbackする() {
        let wf = workflow(
            vec![
                command_node(
                    "route",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "decision".to_string(),
                        cases: BTreeMap::from([
                            ("SHIP".to_string(), "done".to_string()),
                            ("HOLD".to_string(), "fix".to_string()),
                        ]),
                        next: Some("fallback".to_string()),
                    }],
                ),
                session_node("done", None, vec![]),
                session_node("fix", None, vec![]),
                session_node("fallback", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                enum_schema("decision", &["SHIP", "HOLD"]),
            )]),
        );

        assert!(validate_rules(&wf).is_empty());
        assert_eq!(
            route(&wf, 0, Some(&serde_json::json!({})), &HashMap::new()).unwrap(),
            RouteDecision::TransitionTo("fallback".to_string())
        );
    }

    #[test]
    fn test_rules_loop_guard超過でon_exhaustedへ遷移する() {
        let wf = workflow(
            vec![
                session_node("fix", None, vec![Rule::Next("review".to_string())]),
                session_node(
                    "review",
                    None,
                    vec![
                        Rule::LoopGuard {
                            max_iterations: 2,
                            on_exhausted: "give_up".to_string(),
                        },
                        Rule::Next("fix".to_string()),
                    ],
                ),
                session_node("give_up", None, vec![]),
            ],
            BTreeMap::new(),
        );
        assert!(validate_rules(&wf).is_empty());
        assert_eq!(
            route(&wf, 0, None, &HashMap::from([("review".to_string(), 2)])).unwrap(),
            RouteDecision::TransitionTo("give_up".to_string())
        );
    }

    fn cycle_with_exit_guard_workflow(guard_on_cycle: bool) -> WorkflowDefinition {
        let mut node_b_rules = vec![Rule::Next("step_a".to_string())];
        if guard_on_cycle {
            node_b_rules.insert(
                0,
                Rule::LoopGuard {
                    max_iterations: 3,
                    on_exhausted: "done".to_string(),
                },
            );
        }
        workflow(
            vec![
                session_node(
                    "step_a",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "decision".to_string(),
                        cases: BTreeMap::from([
                            ("LOOP".to_string(), "step_b".to_string()),
                            ("EXIT".to_string(), "step_c".to_string()),
                        ]),
                        next: None,
                    }],
                ),
                session_node("step_b", None, node_b_rules),
                session_node(
                    "step_c",
                    None,
                    vec![Rule::LoopGuard {
                        max_iterations: 3,
                        on_exhausted: "done".to_string(),
                    }],
                ),
                session_node("done", None, vec![]),
            ],
            BTreeMap::from([(
                "verdict".to_string(),
                enum_schema("decision", &["LOOP", "EXIT"]),
            )]),
        )
    }

    #[test]
    fn test_rules_cycle_guardは閉路外分岐のloop_guardでは充足しない() {
        let wf = cycle_with_exit_guard_workflow(false);

        let errors = validate_rules(&wf);

        assert!(
            errors.iter().any(|error| matches!(
                error,
                RoutingValidationError::Invalid { step, reason }
                    if step == "step_a" && reason.contains("cycle reachable")
            )),
            "expected step_a cycle guard error, got {errors:?}"
        );
        assert!(
            errors.iter().any(|error| matches!(
                error,
                RoutingValidationError::Invalid { step, reason }
                    if step == "step_b" && reason.contains("cycle reachable")
            )),
            "expected step_b cycle guard error, got {errors:?}"
        );
    }

    #[test]
    fn test_rules_cycle_guardは閉路上nodeにloop_guardがあれば通る() {
        let wf = cycle_with_exit_guard_workflow(true);

        assert!(validate_rules(&wf).is_empty());
    }

    proptest! {
        #[test]
        fn prop_when_boolean_artifact値は必ず一意の遷移先に定まる(passed in any::<bool>()) {
            let wf = workflow(
                vec![
                    session_node(
                        "judge",
                        Some("verdict"),
                        vec![Rule::When {
                            on: "passed".to_string(),
                            then: "done".to_string(),
                            next: "fix".to_string(),
                        }],
                    ),
                    session_node("done", None, vec![]),
                    session_node("fix", None, vec![]),
                ],
                BTreeMap::from([("verdict".to_string(), bool_schema("passed"))]),
            );
            prop_assert!(validate_rules(&wf).is_empty());

            let decision = route(
                &wf,
                0,
                Some(&serde_json::json!({ "passed": passed })),
                &HashMap::new(),
            )
            .unwrap();
            prop_assert_eq!(
                decision,
                RouteDecision::TransitionTo(if passed { "done" } else { "fix" }.to_string())
            );
        }

        #[test]
        fn prop_switch_enum_artifact値は必ず一意の遷移先に定まる(index in 0usize..3) {
            let values = ["LGTM", "NEEDS_FIX", "ESCALATE"];
            let targets = ["done", "fix", "approval"];
            let wf = workflow(
                vec![
                    session_node(
                        "judge",
                        Some("verdict"),
                        vec![Rule::Switch {
                            on: "decision".to_string(),
                            cases: BTreeMap::from([
                                ("LGTM".to_string(), "done".to_string()),
                                ("NEEDS_FIX".to_string(), "fix".to_string()),
                                ("ESCALATE".to_string(), "approval".to_string()),
                            ]),
                            next: None,
                        }],
                    ),
                    session_node("done", None, vec![]),
                    session_node("fix", None, vec![]),
                    session_node("approval", None, vec![]),
                ],
                BTreeMap::from([(
                    "verdict".to_string(),
                    enum_schema("decision", &values),
                )]),
            );
            prop_assert!(validate_rules(&wf).is_empty());

            let decision = route(
                &wf,
                0,
                Some(&serde_json::json!({ "decision": values[index] })),
                &HashMap::new(),
            )
            .unwrap();
            prop_assert_eq!(
                decision,
                RouteDecision::TransitionTo(targets[index].to_string())
            );
        }
    }
}
