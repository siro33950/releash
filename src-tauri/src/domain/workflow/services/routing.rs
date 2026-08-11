use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopGuardResetBaselines {
    by_guarded_node: HashMap<String, u32>,
}

impl LoopGuardResetBaselines {
    pub fn record_successful_completion(
        &mut self,
        workflow: &WorkflowDefinition,
        completed_node_name: &str,
        node_execution_counts: &HashMap<String, u32>,
    ) {
        for guarded_node in &workflow.nodes {
            let Some((_, _, Some(reset_on))) = loop_guard(guarded_node) else {
                continue;
            };
            if reset_on == completed_node_name {
                let cumulative_count = node_execution_counts
                    .get(&guarded_node.name)
                    .copied()
                    .unwrap_or(0);
                self.by_guarded_node
                    .insert(guarded_node.name.clone(), cumulative_count);
            }
        }
    }

    pub fn execution_count(
        &self,
        guarded_node_name: &str,
        cumulative_count: u32,
        reset_on: Option<&str>,
    ) -> u32 {
        if reset_on.is_some() {
            cumulative_count.saturating_sub(
                self.by_guarded_node
                    .get(guarded_node_name)
                    .copied()
                    .unwrap_or(0),
            )
        } else {
            cumulative_count
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingValidationError {
    UnknownRuleTarget {
        node: String,
        target: String,
    },
    UnknownLoopGuardResetNode {
        node: String,
        reset_on: String,
    },
    MultipleDiscriminators {
        node: String,
    },
    MultipleLoopGuards {
        node: String,
    },
    MultipleNextCatchAll {
        node: String,
    },
    StandaloneNextWithDiscriminator {
        node: String,
    },
    WhenFieldNotBoolean {
        node: String,
        field: String,
        reason: Option<String>,
    },
    SwitchFieldNotEnum {
        node: String,
        field: String,
        reason: Option<String>,
    },
    SwitchUnknownCase {
        node: String,
        field: String,
        case: String,
    },
    SwitchMissingCases {
        node: String,
        field: String,
        missing: Vec<String>,
    },
    SwitchExhaustiveHasNext {
        node: String,
    },
    SwitchRequiresNext {
        node: String,
    },
    DiscriminatorOnFanout {
        node: String,
    },
    DiscriminatorWithoutArtifact {
        node: String,
    },
    LoopGuardMaxIterations {
        node: String,
    },
    CycleWithoutLoopGuard {
        node: String,
    },
    UnreachableNode {
        node: String,
    },
    FanoutChildLeafViolation {
        fanout: String,
        child: String,
        reason: String,
    },
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
    errors.extend(validate_fanout_child_leaf_constraints(workflow));
    errors.extend(validate_cycles_have_loop_guard(workflow));

    errors
}

#[cfg(test)]
pub fn route(
    workflow: &WorkflowDefinition,
    current_index: usize,
    artifact: Option<&Value>,
    node_execution_counts: &HashMap<String, u32>,
) -> Result<RouteDecision, WorkflowError> {
    route_with_reset_baselines(
        workflow,
        current_index,
        artifact,
        node_execution_counts,
        &LoopGuardResetBaselines::default(),
    )
}

pub fn route_with_reset_baselines(
    workflow: &WorkflowDefinition,
    current_index: usize,
    artifact: Option<&Value>,
    node_execution_counts: &HashMap<String, u32>,
    loop_guard_reset_baselines: &LoopGuardResetBaselines,
) -> Result<RouteDecision, WorkflowError> {
    let node = workflow.nodes.get(current_index).ok_or_else(|| {
        WorkflowError::validation(format!("node index out of range: {current_index}"))
    })?;

    let Some(target) = raw_target(node, artifact)? else {
        return Ok(RouteDecision::Completed);
    };
    guarded_target_with_reset_baselines(
        workflow,
        target,
        node_execution_counts,
        loop_guard_reset_baselines,
    )
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

pub fn validate_reachability(workflow: &WorkflowDefinition) -> Vec<RoutingValidationError> {
    let reachable = reachable_nodes_from_entry(workflow);
    workflow
        .nodes
        .iter()
        .skip(1)
        .filter(|node| !reachable.contains(node.name.as_str()))
        .map(|node| RoutingValidationError::UnreachableNode {
            node: node.name.clone(),
        })
        .collect()
}

fn reachable_nodes_from_entry(workflow: &WorkflowDefinition) -> HashSet<&str> {
    let node_by_name: BTreeMap<_, _> = workflow
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let Some(entry) = workflow.nodes.first() else {
        return HashSet::new();
    };
    let mut reachable = HashSet::new();
    let fanout_child_names = fanout_child_names(workflow);
    let mut queue = VecDeque::from([entry.name.as_str()]);
    while let Some(current) = queue.pop_front() {
        if !reachable.insert(current) {
            continue;
        }
        let Some(node) = node_by_name.get(current).copied() else {
            continue;
        };
        for target in explicit_targets(node, &fanout_child_names) {
            if node_by_name.contains_key(target) && !reachable.contains(target) {
                queue.push_back(target);
            }
        }
    }
    reachable
}

fn explicit_targets<'a>(
    node: &'a NodeDefinition,
    fanout_child_names: &HashSet<&str>,
) -> Vec<&'a str> {
    if fanout_child_names.contains(node.name.as_str()) {
        return Vec::new();
    }
    let mut targets = node.rules.iter().flat_map(rule_targets).collect::<Vec<_>>();
    if let Some(fanout) = node.fanout() {
        targets.extend(fanout.child.iter().map(String::as_str));
    }
    targets
}

fn fanout_child_names(workflow: &WorkflowDefinition) -> HashSet<&str> {
    workflow
        .nodes
        .iter()
        .filter_map(NodeDefinition::fanout)
        .flat_map(|fanout| fanout.child.iter().map(String::as_str))
        .collect()
}

fn validate_fanout_child_leaf_constraints(
    workflow: &WorkflowDefinition,
) -> Vec<RoutingValidationError> {
    let node_by_name: BTreeMap<_, _> = workflow
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let entry = workflow.nodes.first().map(|node| node.name.as_str());
    let mut parent_by_child = BTreeMap::new();
    let mut errors = Vec::new();

    for parent in &workflow.nodes {
        let Some(fanout) = parent.fanout() else {
            continue;
        };
        for child in &fanout.child {
            parent_by_child
                .entry(child.as_str())
                .or_insert(parent.name.as_str());
            if entry == Some(child.as_str()) {
                errors.push(RoutingValidationError::FanoutChildLeafViolation {
                    fanout: parent.name.clone(),
                    child: child.clone(),
                    reason: "fanout child cannot be the workflow entry node".to_string(),
                });
            }
            if node_by_name
                .get(child.as_str())
                .is_some_and(|node| node.is_fanout())
            {
                errors.push(RoutingValidationError::FanoutChildLeafViolation {
                    fanout: parent.name.clone(),
                    child: child.clone(),
                    reason: "fanout child must be a command or session node".to_string(),
                });
            }
        }
    }

    for source in &workflow.nodes {
        for target in source.rules.iter().flat_map(rule_targets) {
            let Some(parent) = parent_by_child.get(target).copied() else {
                continue;
            };
            errors.push(RoutingValidationError::FanoutChildLeafViolation {
                fanout: parent.to_string(),
                child: target.to_string(),
                reason: format!(
                    "fanout child cannot be a normal transition target from node '{}'",
                    source.name
                ),
            });
        }
    }

    errors
}

#[cfg(test)]
fn reachable_node_names(workflow: &WorkflowDefinition) -> BTreeSet<&str> {
    reachable_nodes_from_entry(workflow).into_iter().collect()
}

pub fn loop_guard(node: &NodeDefinition) -> Option<(u32, &str, Option<&str>)> {
    node.rules.iter().find_map(|rule| match rule {
        Rule::LoopGuard {
            max_iterations,
            on_exhausted,
            reset_on,
        } => Some((*max_iterations, on_exhausted.as_str(), reset_on.as_deref())),
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
                    node: node.name.clone(),
                    target: target.to_string(),
                });
            }
        }
        match rule {
            Rule::When { .. } | Rule::Switch { .. } => discriminator_count += 1,
            Rule::LoopGuard {
                max_iterations,
                reset_on,
                ..
            } => {
                loop_guard_count += 1;
                if *max_iterations == 0 {
                    errors.push(RoutingValidationError::LoopGuardMaxIterations {
                        node: node.name.clone(),
                    });
                }
                if let Some(reset_on) = reset_on {
                    if !node_names.contains(reset_on.as_str()) {
                        errors.push(RoutingValidationError::UnknownLoopGuardResetNode {
                            node: node.name.clone(),
                            reset_on: reset_on.clone(),
                        });
                    }
                }
            }
            Rule::Next(_) => next_count += 1,
        }
    }

    if discriminator_count > 1 {
        errors.push(RoutingValidationError::MultipleDiscriminators {
            node: node.name.clone(),
        });
    }
    if loop_guard_count > 1 {
        errors.push(RoutingValidationError::MultipleLoopGuards {
            node: node.name.clone(),
        });
    }
    if next_count > 1 {
        errors.push(RoutingValidationError::MultipleNextCatchAll {
            node: node.name.clone(),
        });
    }
    if discriminator_count > 0 && next_count > 0 {
        errors.push(RoutingValidationError::StandaloneNextWithDiscriminator {
            node: node.name.clone(),
        });
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
                errors.push(RoutingValidationError::WhenFieldNotBoolean {
                    node: node.name.clone(),
                    field: on.clone(),
                    reason: Some(reason),
                });
            }
        }
        Some(Rule::Switch { on, cases, next }) => {
            match validate_routing_field(workflow, node, on, RoutingFieldKind::Enum) {
                Ok(enum_values) => {
                    let enum_set: BTreeSet<_> = enum_values.iter().map(String::as_str).collect();
                    let case_set: BTreeSet<_> = cases.keys().map(String::as_str).collect();
                    for case in case_set.difference(&enum_set) {
                        errors.push(RoutingValidationError::SwitchUnknownCase {
                            node: node.name.clone(),
                            field: on.clone(),
                            case: (*case).to_string(),
                        });
                    }
                    let missing: Vec<_> = enum_set.difference(&case_set).copied().collect();
                    let needs_p11_next = node.is_command() && node.artifact.is_some() && on != "ok";
                    if missing.is_empty() {
                        if next.is_some() && !needs_p11_next {
                            errors.push(RoutingValidationError::SwitchExhaustiveHasNext {
                                node: node.name.clone(),
                            });
                        }
                        if needs_p11_next && next.is_none() {
                            errors.push(RoutingValidationError::SwitchRequiresNext {
                                node: node.name.clone(),
                            });
                        }
                    } else if next.is_none() {
                        errors.push(RoutingValidationError::SwitchMissingCases {
                            node: node.name.clone(),
                            field: on.clone(),
                            missing: missing.into_iter().map(str::to_string).collect(),
                        });
                    }
                }
                Err(reason) => errors.push(RoutingValidationError::SwitchFieldNotEnum {
                    node: node.name.clone(),
                    field: on.clone(),
                    reason: Some(reason),
                }),
            }
        }
        _ => {}
    }

    if discriminator.is_some() && matches!(node.kind, NodeKind::Fanout(_)) {
        errors.push(RoutingValidationError::DiscriminatorOnFanout {
            node: node.name.clone(),
        });
    }
    if discriminator.is_some() && node.artifact.is_none() && !node.is_command() {
        errors.push(RoutingValidationError::DiscriminatorWithoutArtifact {
            node: node.name.clone(),
        });
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

pub fn guarded_target_with_reset_baselines(
    workflow: &WorkflowDefinition,
    mut target: String,
    node_execution_counts: &HashMap<String, u32>,
    loop_guard_reset_baselines: &LoopGuardResetBaselines,
) -> Result<RouteDecision, WorkflowError> {
    for _ in 0..workflow.nodes.len() {
        let target_node = workflow
            .nodes
            .iter()
            .find(|node| node.name == target)
            .ok_or_else(|| WorkflowError::validation(format!("node not found: {target}")))?;
        let Some((max_iterations, on_exhausted, reset_on)) = loop_guard(target_node) else {
            return Ok(RouteDecision::TransitionTo(target));
        };
        let cumulative_count = node_execution_counts.get(&target).copied().unwrap_or(0);
        let count = loop_guard_reset_baselines.execution_count(&target, cumulative_count, reset_on);
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
    let fanout_child_names = fanout_child_names(workflow);
    for node in &workflow.nodes {
        graph.entry(node.name.as_str()).or_default();
        if fanout_child_names.contains(node.name.as_str()) {
            continue;
        }
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
                    errors.push(RoutingValidationError::CycleWithoutLoopGuard {
                        node: node.name.clone(),
                    });
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

#[cfg(test)]
mod routing_tests {
    use super::*;
    use crate::domain::workflow::value_objects::{
        CommandSpec, FacetRefs, FanoutSpec, NodeKind, SessionSpec,
    };
    use proptest::prelude::*;

    fn bool_schema(field: &str) -> SchemaDef {
        SchemaDef::Object {
            properties: BTreeMap::from([(field.to_string(), SchemaDef::Boolean)]),
            required: BTreeSet::from([field.to_string()]),
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
        }
    }

    fn mixed_schema(fields: Vec<(&str, SchemaDef)>, required: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties: fields
                .into_iter()
                .map(|(name, schema)| (name.to_string(), schema))
                .collect(),
            required: required.iter().map(|value| (*value).to_string()).collect(),
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

    fn fanout_node(name: &str, children: &[&str], rules: Vec<Rule>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: children.iter().map(|child| (*child).to_string()).collect(),
                items: None,
            }),
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
            errors
                .iter()
                .any(|error| routing_error_test_reason(error).contains(expected)),
            "expected invalid reason containing '{expected}', got {errors:?}"
        );
    }

    fn routing_error_test_reason(error: &RoutingValidationError) -> String {
        match error {
            RoutingValidationError::MultipleDiscriminators { .. } => {
                "rules can contain at most one when or switch discriminator".to_string()
            }
            RoutingValidationError::MultipleLoopGuards { .. } => {
                "rules can contain at most one loop_guard".to_string()
            }
            RoutingValidationError::MultipleNextCatchAll { .. } => {
                "rules can contain at most one next catch-all".to_string()
            }
            RoutingValidationError::StandaloneNextWithDiscriminator { .. } => {
                "standalone next cannot be combined with when or switch".to_string()
            }
            RoutingValidationError::WhenFieldNotBoolean { reason, .. }
            | RoutingValidationError::SwitchFieldNotEnum { reason, .. } => {
                reason.clone().unwrap_or_default()
            }
            RoutingValidationError::SwitchUnknownCase { field, case, .. } => {
                format!("switch case '{case}' is not declared in enum field '{field}'")
            }
            RoutingValidationError::SwitchMissingCases { field, missing, .. } => format!(
                "switch on '{field}' is missing enum cases [{}] and requires next",
                missing.join(", ")
            ),
            RoutingValidationError::SwitchExhaustiveHasNext { .. } => {
                "exhaustive switch cannot also define next catch-all".to_string()
            }
            RoutingValidationError::SwitchRequiresNext { .. } => {
                "command artifact routing on Contract field requires next catch-all".to_string()
            }
            RoutingValidationError::DiscriminatorOnFanout { .. } => {
                "fanout nodes cannot use when or switch rules".to_string()
            }
            RoutingValidationError::DiscriminatorWithoutArtifact { .. } => {
                "nodes without an artifact cannot use when or switch rules".to_string()
            }
            RoutingValidationError::LoopGuardMaxIterations { .. } => {
                "loop_guard.max_iterations must be greater than 0".to_string()
            }
            RoutingValidationError::CycleWithoutLoopGuard { .. } => {
                "cycle reachable from this node has no loop_guard on cycle nodes".to_string()
            }
            RoutingValidationError::FanoutChildLeafViolation { reason, .. } => reason.clone(),
            RoutingValidationError::UnknownRuleTarget { .. }
            | RoutingValidationError::UnknownLoopGuardResetNode { .. }
            | RoutingValidationError::UnreachableNode { .. } => String::new(),
        }
    }

    #[test]
    fn fanout_edges_reach_children_but_child_rules_do_not_escape_leaf_scope() {
        let wf = workflow(
            vec![
                fanout_node("fanout", &["worker"], vec![Rule::Next("done".to_string())]),
                session_node("worker", None, vec![Rule::Next("fanout".to_string())]),
                session_node("done", None, vec![]),
            ],
            BTreeMap::new(),
        );

        assert!(validate_rules(&wf).is_empty());
        assert_eq!(
            reachable_node_names(&wf),
            BTreeSet::from(["done", "fanout", "worker"])
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
        assert_eq!(
            route(&wf, 0, Some(&serde_json::json!({})), &HashMap::new()).unwrap(),
            RouteDecision::TransitionTo("fix".to_string()),
            "a missing when field is a no-match and must use sibling next"
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
                            reset_on: None,
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

    #[test]
    fn test_loop_guard_reset_on正常完了ごとに新しいカウント範囲を開始する() {
        let wf = workflow(
            vec![
                session_node("round", None, vec![Rule::Next("fix".to_string())]),
                session_node(
                    "fix",
                    None,
                    vec![
                        Rule::LoopGuard {
                            max_iterations: 2,
                            on_exhausted: "give_up".to_string(),
                            reset_on: Some("round".to_string()),
                        },
                        Rule::Next("round".to_string()),
                    ],
                ),
                session_node("give_up", None, vec![]),
            ],
            BTreeMap::new(),
        );
        let mut counts = HashMap::from([("fix".to_string(), 2)]);
        let mut baselines = LoopGuardResetBaselines::default();

        assert_eq!(
            guarded_target_with_reset_baselines(&wf, "fix".to_string(), &counts, &baselines,)
                .unwrap(),
            RouteDecision::TransitionTo("give_up".to_string()),
            "reset_on が未到達なら Workflow 開始からの累計を使う"
        );

        baselines.record_successful_completion(&wf, "round", &counts);
        assert_eq!(
            guarded_target_with_reset_baselines(&wf, "fix".to_string(), &counts, &baselines,)
                .unwrap(),
            RouteDecision::TransitionTo("fix".to_string())
        );

        counts.insert("fix".to_string(), 3);
        assert_eq!(
            guarded_target_with_reset_baselines(&wf, "fix".to_string(), &counts, &baselines,)
                .unwrap(),
            RouteDecision::TransitionTo("fix".to_string())
        );
        counts.insert("fix".to_string(), 4);
        assert_eq!(
            guarded_target_with_reset_baselines(&wf, "fix".to_string(), &counts, &baselines,)
                .unwrap(),
            RouteDecision::TransitionTo("give_up".to_string())
        );

        baselines.record_successful_completion(&wf, "round", &counts);
        assert_eq!(
            guarded_target_with_reset_baselines(&wf, "fix".to_string(), &counts, &baselines,)
                .unwrap(),
            RouteDecision::TransitionTo("fix".to_string()),
            "2 回目の正常完了でも新しい範囲を開始する"
        );
    }

    #[test]
    fn test_loop_guard_reset_onはcontrol_flow_edgeとして扱わない() {
        let wf = workflow(
            vec![
                session_node("entry", None, vec![Rule::Next("fix".to_string())]),
                session_node(
                    "fix",
                    None,
                    vec![Rule::LoopGuard {
                        max_iterations: 2,
                        on_exhausted: "done".to_string(),
                        reset_on: Some("boundary".to_string()),
                    }],
                ),
                session_node("boundary", None, vec![]),
                session_node("done", None, vec![]),
            ],
            BTreeMap::new(),
        );

        assert!(!reachable_node_names(&wf).contains("boundary"));
    }

    #[test]
    fn reachability_starts_at_entry_and_does_not_seed_unreachable_subgraph_edges() {
        let wf = workflow(
            vec![
                session_node("entry", None, vec![]),
                session_node("orphan", None, vec![Rule::Next("target".to_string())]),
                session_node("target", None, vec![]),
            ],
            BTreeMap::new(),
        );

        assert_eq!(
            reachable_node_names(&wf),
            BTreeSet::from(["entry"]),
            "unreachable subgraph edges must not mark their targets reachable"
        );
        let errors = validate_reachability(&wf);
        for node in ["orphan", "target"] {
            assert!(
                errors.iter().any(|error| matches!(
                    error,
                    RoutingValidationError::UnreachableNode { node: found } if found == node
                )),
                "expected unreachable error for {node}, got {errors:?}"
            );
        }
    }

    #[test]
    fn reachability_follows_explicit_rule_targets_from_reachable_nodes() {
        let wf = workflow(
            vec![
                session_node("entry", None, vec![Rule::Next("target".to_string())]),
                session_node("orphan", None, vec![]),
                session_node("target", None, vec![]),
            ],
            BTreeMap::new(),
        );

        assert_eq!(
            reachable_node_names(&wf),
            BTreeSet::from(["entry", "target"])
        );
        let errors = validate_reachability(&wf);
        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            RoutingValidationError::UnreachableNode { node } if node == "orphan"
        ));
    }

    fn cycle_with_exit_guard_workflow(guard_on_cycle: bool) -> WorkflowDefinition {
        let mut node_b_rules = vec![Rule::Next("node_a".to_string())];
        if guard_on_cycle {
            node_b_rules.insert(
                0,
                Rule::LoopGuard {
                    max_iterations: 3,
                    on_exhausted: "done".to_string(),
                    reset_on: None,
                },
            );
        }
        workflow(
            vec![
                session_node(
                    "node_a",
                    Some("verdict"),
                    vec![Rule::Switch {
                        on: "decision".to_string(),
                        cases: BTreeMap::from([
                            ("LOOP".to_string(), "node_b".to_string()),
                            ("EXIT".to_string(), "node_c".to_string()),
                        ]),
                        next: None,
                    }],
                ),
                session_node("node_b", None, node_b_rules),
                session_node(
                    "node_c",
                    None,
                    vec![Rule::LoopGuard {
                        max_iterations: 3,
                        on_exhausted: "done".to_string(),
                        reset_on: None,
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
                RoutingValidationError::CycleWithoutLoopGuard { node }
                    if node == "node_a"
            )),
            "expected node_a cycle guard error, got {errors:?}"
        );
        assert!(
            errors.iter().any(|error| matches!(
                error,
                RoutingValidationError::CycleWithoutLoopGuard { node }
                    if node == "node_b"
            )),
            "expected node_b cycle guard error, got {errors:?}"
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
