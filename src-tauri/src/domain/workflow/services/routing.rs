use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::domain::workflow::services::contract_schema::{self, RoutingFieldKind};
use crate::domain::workflow::value_objects::{
    ChildEntry, EffectiveRules, NodeDefinition, NodeKind, Rule, SchemaDef, SequenceSpec,
    WorkflowDefinition,
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
        // loop_guard は実行スコープ（root sequence）の children エントリに属する。
        let Some(sequence) = workflow.root_sequence() else {
            return;
        };
        for entry in &sequence.children {
            let Some((_, _, Some(reset_on))) = entry_loop_guard(entry) else {
                continue;
            };
            if reset_on == completed_node_name {
                let cumulative_count = node_execution_counts.get(&entry.name).copied().unwrap_or(0);
                self.by_guarded_node
                    .insert(entry.name.clone(), cumulative_count);
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
    /// children エントリが存在しないカタログ node を参照している。
    UnknownChildReference {
        composite: String,
        child: String,
    },
    /// 同一合成子の children が同じカタログ node を複数回参照している。
    DuplicateChildReference {
        composite: String,
        child: String,
    },
    /// 合成子の children が空。
    EmptyChildren {
        composite: String,
    },
    /// sequence の entry が children のエントリ名を指していない。
    SequenceEntryNotChild {
        sequence: String,
        entry: String,
    },
    /// sequence の output が children のエントリ名を指していない。
    SequenceOutputNotChild {
        sequence: String,
        output: String,
    },
    /// fanout の children エントリに rules が書かれている（fanout に辺は無い）。
    RulesOnFanoutChildEntry {
        fanout: String,
        child: String,
    },
    /// 合成子の子参照の一意性・root 参照禁止などの構造制約違反。
    ChildReferenceViolation {
        composite: String,
        child: String,
        reason: String,
    },
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
}

/// 検証対象の合成子スコープ。検証は全合成子（ネスト含む）に対して行い、
/// 実行（route）は root sequence の1段のみを使う。
struct CompositeScope<'a> {
    owner: &'a NodeDefinition,
    children: &'a [ChildEntry],
    sequence: Option<&'a SequenceSpec>,
}

fn composite_scopes(workflow: &WorkflowDefinition) -> Vec<CompositeScope<'_>> {
    workflow
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Sequence(sequence) => Some(CompositeScope {
                owner: node,
                children: &sequence.children,
                sequence: Some(sequence),
            }),
            NodeKind::Fanout(fanout) => Some(CompositeScope {
                owner: node,
                children: &fanout.children,
                sequence: None,
            }),
            _ => None,
        })
        .collect()
}

pub fn validate_rules(workflow: &WorkflowDefinition) -> Vec<RoutingValidationError> {
    let node_by_name: BTreeMap<_, _> = workflow
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let scopes = composite_scopes(workflow);
    let mut errors = Vec::new();

    // 子参照の帰属表（cross-composite 制約用）。
    let mut composites_by_child: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for scope in &scopes {
        for entry in scope.children {
            composites_by_child
                .entry(entry.name.as_str())
                .or_default()
                .push(scope.owner.name.as_str());
        }
    }

    for scope in &scopes {
        errors.extend(validate_scope_children(workflow, scope, &node_by_name));
    }

    // 子参照の一意性: 同じ node を複数の合成子が子として扱うと、配線の帰属・
    // attempt カウント・loop_guard baseline のキーが曖昧になる。
    for (child, composites) in &composites_by_child {
        if composites.len() > 1 {
            for composite in &composites[1..] {
                errors.push(RoutingValidationError::ChildReferenceViolation {
                    composite: (*composite).to_string(),
                    child: (*child).to_string(),
                    reason: format!(
                        "node '{child}' is already a child of composite '{}'",
                        composites[0]
                    ),
                });
            }
        }
    }

    // root は合成子の子になれない。
    if let Some(composites) = composites_by_child.get(workflow.entry.as_str()) {
        for composite in composites {
            errors.push(RoutingValidationError::ChildReferenceViolation {
                composite: (*composite).to_string(),
                child: workflow.entry.clone(),
                reason: "the workflow root node cannot be a composite child".to_string(),
            });
        }
    }

    // rules ターゲットは、同一スコープの子か、どの合成子の子でもない node に限る。
    for scope in &scopes {
        let own_children: BTreeSet<_> = scope.children.iter().map(|c| c.name.as_str()).collect();
        for entry in scope.children {
            let Some(rules) = &entry.rules else { continue };
            for target in rules.iter().flat_map(rule_targets) {
                if own_children.contains(target) {
                    continue;
                }
                let Some(owners) = composites_by_child.get(target) else {
                    continue;
                };
                if let Some(owner) = owners.first() {
                    errors.push(RoutingValidationError::ChildReferenceViolation {
                        composite: (*owner).to_string(),
                        child: target.to_string(),
                        reason: format!(
                            "a child of composite '{owner}' cannot be a transition target from '{}'",
                            entry.name
                        ),
                    });
                }
            }
        }
    }

    for scope in &scopes {
        if let Some(sequence) = scope.sequence {
            errors.extend(validate_scope_cycles(&scope.owner.name, sequence));
        }
    }

    errors
}

fn validate_scope_children(
    workflow: &WorkflowDefinition,
    scope: &CompositeScope<'_>,
    node_by_name: &BTreeMap<&str, &NodeDefinition>,
) -> Vec<RoutingValidationError> {
    let composite = scope.owner.name.as_str();
    let mut errors = Vec::new();

    if scope.children.is_empty() {
        errors.push(RoutingValidationError::EmptyChildren {
            composite: composite.to_string(),
        });
        return errors;
    }

    let mut seen = BTreeSet::new();
    for entry in scope.children {
        if !seen.insert(entry.name.as_str()) {
            errors.push(RoutingValidationError::DuplicateChildReference {
                composite: composite.to_string(),
                child: entry.name.clone(),
            });
        }
        if !node_by_name.contains_key(entry.name.as_str()) {
            errors.push(RoutingValidationError::UnknownChildReference {
                composite: composite.to_string(),
                child: entry.name.clone(),
            });
        }
    }

    match scope.sequence {
        Some(sequence) => {
            if let Some(entry_name) = &sequence.entry {
                if !seen.contains(entry_name.as_str()) {
                    errors.push(RoutingValidationError::SequenceEntryNotChild {
                        sequence: composite.to_string(),
                        entry: entry_name.clone(),
                    });
                }
            }
            if let Some(output) = &sequence.output {
                if !seen.contains(output.as_str()) {
                    errors.push(RoutingValidationError::SequenceOutputNotChild {
                        sequence: composite.to_string(),
                        output: output.clone(),
                    });
                }
            }
            for entry in scope.children {
                let Some(rules) = &entry.rules else { continue };
                let child = node_by_name.get(entry.name.as_str()).copied();
                errors.extend(validate_entry_rules(
                    workflow,
                    &entry.name,
                    rules,
                    child,
                    node_by_name,
                ));
            }
        }
        None => {
            for entry in scope.children {
                if entry.rules.is_some() {
                    errors.push(RoutingValidationError::RulesOnFanoutChildEntry {
                        fanout: composite.to_string(),
                        child: entry.name.clone(),
                    });
                }
            }
        }
    }

    errors
}

/// children エントリの rules の排他・網羅・ループ健全性・型を検証する。
/// 判別（when / switch）の型検証はエントリが参照する子 node の artifact Contract
/// に対して行う。
fn validate_entry_rules(
    workflow: &WorkflowDefinition,
    entry_name: &str,
    rules: &[Rule],
    child: Option<&NodeDefinition>,
    node_by_name: &BTreeMap<&str, &NodeDefinition>,
) -> Vec<RoutingValidationError> {
    let mut errors = Vec::new();
    let mut discriminator_count = 0usize;
    let mut loop_guard_count = 0usize;
    let mut next_count = 0usize;

    for rule in rules {
        for target in rule_targets(rule) {
            if !node_by_name.contains_key(target) {
                errors.push(RoutingValidationError::UnknownRuleTarget {
                    node: entry_name.to_string(),
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
                        node: entry_name.to_string(),
                    });
                }
                if let Some(reset_on) = reset_on {
                    if !node_by_name.contains_key(reset_on.as_str()) {
                        errors.push(RoutingValidationError::UnknownLoopGuardResetNode {
                            node: entry_name.to_string(),
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
            node: entry_name.to_string(),
        });
    }
    if loop_guard_count > 1 {
        errors.push(RoutingValidationError::MultipleLoopGuards {
            node: entry_name.to_string(),
        });
    }
    if next_count > 1 {
        errors.push(RoutingValidationError::MultipleNextCatchAll {
            node: entry_name.to_string(),
        });
    }
    if discriminator_count > 0 && next_count > 0 {
        errors.push(RoutingValidationError::StandaloneNextWithDiscriminator {
            node: entry_name.to_string(),
        });
    }

    let discriminator = rules
        .iter()
        .find(|rule| matches!(rule, Rule::When { .. } | Rule::Switch { .. }));
    let Some(child) = child else {
        // 参照先不明は UnknownChildReference 側で報告済み。型検証は行えない。
        return errors;
    };
    match discriminator {
        Some(Rule::When { on, .. }) => {
            if let Err(reason) =
                validate_routing_field(workflow, child, on, RoutingFieldKind::Boolean)
            {
                errors.push(RoutingValidationError::WhenFieldNotBoolean {
                    node: entry_name.to_string(),
                    field: on.clone(),
                    reason: Some(reason),
                });
            }
        }
        Some(Rule::Switch { on, cases, next }) => {
            match validate_routing_field(workflow, child, on, RoutingFieldKind::Enum) {
                Ok(enum_values) => {
                    let enum_set: BTreeSet<_> = enum_values.iter().map(String::as_str).collect();
                    let case_set: BTreeSet<_> = cases.keys().map(String::as_str).collect();
                    for case in case_set.difference(&enum_set) {
                        errors.push(RoutingValidationError::SwitchUnknownCase {
                            node: entry_name.to_string(),
                            field: on.clone(),
                            case: (*case).to_string(),
                        });
                    }
                    let missing: Vec<_> = enum_set.difference(&case_set).copied().collect();
                    let needs_p11_next =
                        child.is_command() && child.artifact.is_some() && on != "ok";
                    if missing.is_empty() {
                        if next.is_some() && !needs_p11_next {
                            errors.push(RoutingValidationError::SwitchExhaustiveHasNext {
                                node: entry_name.to_string(),
                            });
                        }
                        if needs_p11_next && next.is_none() {
                            errors.push(RoutingValidationError::SwitchRequiresNext {
                                node: entry_name.to_string(),
                            });
                        }
                    } else if next.is_none() {
                        errors.push(RoutingValidationError::SwitchMissingCases {
                            node: entry_name.to_string(),
                            field: on.clone(),
                            missing: missing.into_iter().map(str::to_string).collect(),
                        });
                    }
                }
                Err(reason) => errors.push(RoutingValidationError::SwitchFieldNotEnum {
                    node: entry_name.to_string(),
                    field: on.clone(),
                    reason: Some(reason),
                }),
            }
        }
        _ => {}
    }

    if discriminator.is_some() && child.is_fanout() {
        errors.push(RoutingValidationError::DiscriminatorOnFanout {
            node: entry_name.to_string(),
        });
    }
    if discriminator.is_some() && child.artifact.is_none() && !child.is_command() {
        errors.push(RoutingValidationError::DiscriminatorWithoutArtifact {
            node: entry_name.to_string(),
        });
    }

    errors
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

/// 到達可能性は2層で検証する。
/// (a) 各 sequence スコープ内: 実効 entry から実効辺で辿れない子。
/// (b) カタログレベル: root からの構造参照（children / rules ターゲット）の閉包に
///     入らない node（ネスト合成子配下も構造参照として辿る）。
pub fn validate_reachability(workflow: &WorkflowDefinition) -> Vec<RoutingValidationError> {
    let mut unreachable: BTreeSet<String> = BTreeSet::new();

    let referenced = catalog_reference_closure(workflow);
    for node in &workflow.nodes {
        if node.name != workflow.entry && !referenced.contains(node.name.as_str()) {
            unreachable.insert(node.name.clone());
        }
    }

    for node in &workflow.nodes {
        let Some(sequence) = node.sequence() else {
            continue;
        };
        for name in scope_flow_unreachable(sequence) {
            unreachable.insert(name.to_string());
        }
    }

    unreachable
        .into_iter()
        .map(|node| RoutingValidationError::UnreachableNode { node })
        .collect()
}

fn catalog_reference_closure(workflow: &WorkflowDefinition) -> HashSet<&str> {
    let node_by_name: BTreeMap<_, _> = workflow
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::from([workflow.entry.as_str()]);
    while let Some(current) = queue.pop_front() {
        if !reachable.insert(current) {
            continue;
        }
        let Some(node) = node_by_name.get(current).copied() else {
            continue;
        };
        let children = match &node.kind {
            NodeKind::Sequence(sequence) => Some(&sequence.children),
            NodeKind::Fanout(fanout) => Some(&fanout.children),
            _ => None,
        };
        let Some(children) = children else { continue };
        for entry in children {
            if node_by_name.contains_key(entry.name.as_str()) {
                queue.push_back(entry.name.as_str());
            }
            for target in entry.rules.iter().flatten().flat_map(rule_targets) {
                if node_by_name.contains_key(target) {
                    queue.push_back(target);
                }
            }
        }
    }
    reachable
}

fn scope_flow_unreachable(sequence: &SequenceSpec) -> Vec<&str> {
    let child_names: BTreeSet<_> = sequence
        .children
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    let Some(entry_name) = sequence.entry_child_name() else {
        return Vec::new();
    };
    if !child_names.contains(entry_name) {
        // entry が children 外なら SequenceEntryNotChild 側で報告する。
        return Vec::new();
    }
    let mut reached = BTreeSet::new();
    let mut queue = VecDeque::from([entry_name]);
    while let Some(current) = queue.pop_front() {
        if !reached.insert(current) {
            continue;
        }
        match sequence.effective_rules(current) {
            EffectiveRules::AdjacentNext(next) => queue.push_back(next),
            EffectiveRules::Rules(rules) => {
                for target in rules.iter().flat_map(rule_targets) {
                    if child_names.contains(target) {
                        queue.push_back(target);
                    }
                }
            }
            EffectiveRules::Terminal => {}
        }
    }
    child_names
        .into_iter()
        .filter(|name| !reached.contains(name))
        .collect()
}

/// children エントリの明示 rules から loop_guard を取り出す。
pub(crate) fn entry_loop_guard(entry: &ChildEntry) -> Option<(u32, &str, Option<&str>)> {
    entry.rules.as_ref()?.iter().find_map(|rule| match rule {
        Rule::LoopGuard {
            max_iterations,
            on_exhausted,
            reset_on,
        } => Some((*max_iterations, on_exhausted.as_str(), reset_on.as_deref())),
        _ => None,
    })
}

fn root_entry_loop_guard<'a>(
    workflow: &'a WorkflowDefinition,
    node_name: &str,
) -> Option<(u32, &'a str, Option<&'a str>)> {
    let sequence = workflow.root_sequence()?;
    entry_loop_guard(sequence.child_entry(node_name)?)
}

fn validate_scope_cycles(owner: &str, sequence: &SequenceSpec) -> Vec<RoutingValidationError> {
    let _ = owner;
    let child_names: BTreeSet<&str> = sequence
        .children
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for entry in &sequence.children {
        graph.entry(entry.name.as_str()).or_default();
        match sequence.effective_rules(&entry.name) {
            EffectiveRules::AdjacentNext(next) => {
                graph.entry(entry.name.as_str()).or_default().insert(next);
            }
            EffectiveRules::Rules(rules) => {
                for target in rules.iter().flat_map(rule_targets) {
                    if child_names.contains(target) {
                        graph.entry(entry.name.as_str()).or_default().insert(target);
                    }
                }
            }
            EffectiveRules::Terminal => {}
        }
    }

    let entry_by_name: BTreeMap<&str, &ChildEntry> = sequence
        .children
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();
    let mut errors = Vec::new();
    for component in cyclic_strongly_connected_components(&child_names, &graph) {
        let has_guard_on_cycle = component
            .iter()
            .filter_map(|name| entry_by_name.get(*name).copied())
            .any(|entry| entry_loop_guard(entry).is_some());
        if !has_guard_on_cycle {
            for name in component {
                errors.push(RoutingValidationError::CycleWithoutLoopGuard {
                    node: name.to_string(),
                });
            }
        }
    }
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

    // 配線は root sequence の children エントリが持つ。root が leaf / fanout の
    // 場合は単独実行（辺なし = 完了）。
    let target = match workflow.root_sequence() {
        None => None,
        Some(sequence) => match sequence.effective_rules(&node.name) {
            EffectiveRules::Rules(rules) => raw_target(&node.name, rules, artifact)?,
            EffectiveRules::AdjacentNext(next) => Some(next.to_string()),
            EffectiveRules::Terminal => None,
        },
    };
    let Some(target) = target else {
        return Ok(RouteDecision::Completed);
    };
    guarded_target_with_reset_baselines(
        workflow,
        target,
        node_execution_counts,
        loop_guard_reset_baselines,
    )
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
    node_name: &str,
    rules: &[Rule],
    artifact: Option<&Value>,
) -> Result<Option<String>, WorkflowError> {
    let discriminator = rules
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
                    "No matching switch case for node '{node_name}' and no next catch-all"
                ))
            })
        }
        _ => Ok(rules.iter().find_map(|rule| match rule {
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
        if workflow.node_by_name(&target).is_none() {
            return Err(WorkflowError::validation(format!(
                "node not found: {target}"
            )));
        }
        let Some((max_iterations, on_exhausted, reset_on)) =
            root_entry_loop_guard(workflow, &target)
        else {
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
        CommandSpec, FanoutSpec, InputSourceRef, NodeKind, SequenceSpec,
    };

    fn command_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: "echo hi".to_string(),
            }),
            artifact: None,
            input: Vec::new(),
            completion: Default::default(),
            worktree: None,
        }
    }

    fn sequence_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Sequence(SequenceSpec {
                entry: None,
                output: None,
                children,
            }),
            artifact: None,
            input: Vec::new(),
            completion: Default::default(),
            worktree: None,
        }
    }

    fn fanout_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                children,
                items: None,
            }),
            artifact: None,
            input: Vec::new(),
            completion: Default::default(),
            worktree: None,
        }
    }

    fn entry_with_rules(name: &str, rules: Vec<Rule>) -> ChildEntry {
        ChildEntry {
            name: name.to_string(),
            inputs: Vec::new(),
            rules: Some(rules),
        }
    }

    fn workflow(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            nodes,
            ..Default::default()
        }
    }

    fn node_index(workflow: &WorkflowDefinition, name: &str) -> usize {
        workflow
            .nodes
            .iter()
            .position(|node| node.name == name)
            .expect("node exists")
    }

    fn route_from(
        workflow: &WorkflowDefinition,
        name: &str,
        artifact: Option<&Value>,
    ) -> RouteDecision {
        route(
            workflow,
            node_index(workflow, name),
            artifact,
            &HashMap::new(),
        )
        .expect("route succeeds")
    }

    #[test]
    fn test_隣接辺_rules無しエントリはリストの次へ進み末尾で完了する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("first"),
                    ChildEntry::reference("second"),
                ],
            ),
            command_node("first"),
            command_node("second"),
        ]);

        assert_eq!(
            route_from(&wf, "first", None),
            RouteDecision::TransitionTo("second".to_string())
        );
        assert_eq!(route_from(&wf, "second", None), RouteDecision::Completed);
    }

    #[test]
    fn test_明示終端_空rulesは隣接辺を持たず完了する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("first", Vec::new()),
                    ChildEntry::reference("second"),
                ],
            ),
            command_node("first"),
            command_node("second"),
        ]);

        assert_eq!(route_from(&wf, "first", None), RouteDecision::Completed);
    }

    #[test]
    fn test_children外ターゲット_遷移後は出る辺が無く完了する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![entry_with_rules(
                    "first",
                    vec![Rule::Next("target_only".to_string())],
                )],
            ),
            command_node("first"),
            command_node("target_only"),
        ]);

        assert_eq!(
            route_from(&wf, "first", None),
            RouteDecision::TransitionTo("target_only".to_string())
        );
        assert_eq!(
            route_from(&wf, "target_only", None),
            RouteDecision::Completed
        );
    }

    #[test]
    fn test_単独実行_rootがleafなら完了する() {
        let wf = workflow(vec![command_node("main")]);
        assert_eq!(route_from(&wf, "main", None), RouteDecision::Completed);
    }

    #[test]
    fn test_loop_guard_上限到達でon_exhaustedへ迂回する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("work", vec![Rule::Next("retry".to_string())]),
                    entry_with_rules(
                        "retry",
                        vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "done".to_string(),
                                reset_on: None,
                            },
                            Rule::Next("work".to_string()),
                        ],
                    ),
                    ChildEntry::reference("done"),
                ],
            ),
            command_node("work"),
            command_node("retry"),
            command_node("done"),
        ]);

        let mut counts = HashMap::new();
        counts.insert("retry".to_string(), 1u32);
        let decision = route(&wf, node_index(&wf, "work"), None, &counts).unwrap();
        assert_eq!(decision, RouteDecision::TransitionTo("retry".to_string()));

        counts.insert("retry".to_string(), 2u32);
        let decision = route(&wf, node_index(&wf, "work"), None, &counts).unwrap();
        assert_eq!(decision, RouteDecision::TransitionTo("done".to_string()));
    }

    #[test]
    fn test_reset_baselines_リセット後のカウントで上限を判定する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("work", vec![Rule::Next("retry".to_string())]),
                    entry_with_rules(
                        "retry",
                        vec![
                            Rule::LoopGuard {
                                max_iterations: 1,
                                on_exhausted: "done".to_string(),
                                reset_on: Some("work".to_string()),
                            },
                            Rule::Next("work".to_string()),
                        ],
                    ),
                    ChildEntry::reference("done"),
                ],
            ),
            command_node("work"),
            command_node("retry"),
            command_node("done"),
        ]);

        let mut counts = HashMap::new();
        counts.insert("retry".to_string(), 1u32);

        let mut baselines = LoopGuardResetBaselines::default();
        baselines.record_successful_completion(&wf, "work", &counts);
        let decision =
            route_with_reset_baselines(&wf, node_index(&wf, "work"), None, &counts, &baselines)
                .unwrap();
        assert_eq!(decision, RouteDecision::TransitionTo("retry".to_string()));
    }

    #[test]
    fn test_検証_同一合成子への重複子参照を拒否する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("first"),
                    ChildEntry::reference("first"),
                ],
            ),
            command_node("first"),
        ]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::DuplicateChildReference { child, .. } if child == "first"
        )));
    }

    #[test]
    fn test_検証_未知の子参照を拒否する() {
        let wf = workflow(vec![sequence_node(
            "main",
            vec![ChildEntry::reference("ghost")],
        )]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::UnknownChildReference { child, .. } if child == "ghost"
        )));
    }

    #[test]
    fn test_検証_fanout子エントリのrulesを拒否する() {
        let wf = workflow(vec![
            sequence_node("main", vec![ChildEntry::reference("fan")]),
            fanout_node(
                "fan",
                vec![entry_with_rules(
                    "worker",
                    vec![Rule::Next("worker".to_string())],
                )],
            ),
            command_node("worker"),
        ]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::RulesOnFanoutChildEntry { child, .. } if child == "worker"
        )));
    }

    #[test]
    fn test_検証_他合成子の子への遷移ターゲットを拒否する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("first", vec![Rule::Next("worker".to_string())]),
                    ChildEntry::reference("fan"),
                ],
            ),
            command_node("first"),
            fanout_node("fan", vec![ChildEntry::reference("worker")]),
            command_node("worker"),
        ]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::ChildReferenceViolation { child, .. } if child == "worker"
        )));
    }

    #[test]
    fn test_検証_隣接辺を含む閉路にloop_guardが無ければ拒否する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    ChildEntry::reference("first"),
                    entry_with_rules("second", vec![Rule::Next("first".to_string())]),
                ],
            ),
            command_node("first"),
            command_node("second"),
        ]);

        assert!(validate_rules(&wf)
            .iter()
            .any(|error| matches!(error, RoutingValidationError::CycleWithoutLoopGuard { .. })));
    }

    #[test]
    fn test_検証_entryがchildren外ならエラー() {
        let mut seq = SequenceSpec {
            entry: Some("outside".to_string()),
            output: None,
            children: vec![ChildEntry::reference("first")],
        };
        let wf = workflow(vec![
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(std::mem::take(&mut seq)),
                artifact: None,
                input: Vec::new(),
                completion: Default::default(),
                worktree: None,
            },
            command_node("first"),
            command_node("outside"),
        ]);

        assert!(validate_rules(&wf).iter().any(|error| matches!(
            error,
            RoutingValidationError::SequenceEntryNotChild { entry, .. } if entry == "outside"
        )));
    }

    #[test]
    fn test_到達可能性_スコープ内で辿れない子とカタログ未参照nodeを検出する() {
        let wf = workflow(vec![
            sequence_node(
                "main",
                vec![
                    entry_with_rules("first", Vec::new()),
                    ChildEntry::reference("orphan_child"),
                ],
            ),
            command_node("first"),
            command_node("orphan_child"),
            command_node("unreferenced"),
        ]);

        let unreachable: Vec<_> = validate_reachability(&wf)
            .into_iter()
            .map(|error| match error {
                RoutingValidationError::UnreachableNode { node } => node,
                other => panic!("unexpected error: {other:?}"),
            })
            .collect();
        assert!(unreachable.contains(&"orphan_child".to_string()));
        assert!(unreachable.contains(&"unreferenced".to_string()));
        assert!(!unreachable.contains(&"first".to_string()));
    }

    #[test]
    fn test_パラメータ配線_供給元参照のroot分解() {
        let source = InputSourceRef::new("collect_inputs.spec_dir");
        assert_eq!(source.root(), "collect_inputs");
        assert_eq!(source.field(), Some("spec_dir"));
    }
}
