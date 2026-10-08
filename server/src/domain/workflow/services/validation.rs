use crate::domain::workflow::services::{contract_schema, reference, routing};
use crate::domain::workflow::value_objects::{MAX_FANOUT_CHILDREN, MAX_NODES_PER_WORKFLOW};
use crate::domain::workflow::{
    is_reserved_node_name, FieldPath, InputParam, ItemsSource, NodeDefinition, NodeKind,
    NodeKindName, SchemaDef, WorkflowDefinition, WorkflowDefinitionName, WorkflowError,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSchemaKind {
    InvalidDeclaration,
    UnknownSchemaReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidArtifactReferenceKind {
    ReservedArtifactName,
    UnknownParameter,
    UnknownField,
    InvalidInputRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidEnvironmentReferenceKind {
    UnknownParameter,
    UnknownField,
    InvalidInputRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidRuleKind {
    MultipleDiscriminators,
    MultipleLoopGuards,
    MultipleNextCatchAll,
    StandaloneNextWithDiscriminator,
    WhenFieldNotBoolean,
    SwitchFieldNotEnum,
    SwitchUnknownCase,
    SwitchMissingCases,
    SwitchExhaustiveHasNext,
    SwitchRequiresNext,
    DiscriminatorWithoutArtifact,
    LoopGuardMaxIterations,
    CycleWithoutLoopGuard,
}

/// children エントリの inputs 配線の不正種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputWiringKind {
    /// 供給元が兄弟 node / 自合成子のパラメータ / request / items のどれにも解決しない。
    UnknownSource,
    /// 供給元の記述が不正（空・空白・空の段・request / items への field）。
    InvalidSourceFormat,
    /// 配線先のパラメータ名が子 node の input 宣言に無い。
    UnknownParameter,
    /// items 供給元は items を宣言した fanout の children でのみ使える。
    ItemsUnavailable,
    /// 供給元の兄弟 node が Artifact を産出しない。
    UnavailableSourceArtifact,
    /// 供給元の field パスが Artifact Contract / パラメータ Contract に無い。
    UnknownSourceField,
    /// 供給元名が兄弟 node 名と自合成子のパラメータ名の両方に一致して曖昧。
    AmbiguousSource,
}

/// children エントリの inputs 配線違反の詳細。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputWiringViolation {
    pub node: String,
    pub child: String,
    pub parameter: String,
    pub source: String,
    pub kind: InputWiringKind,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidDelegateKind {
    UnsupportedNodeKind,
    MissingArtifactContract,
    ChildWithoutArtifact,
    MaxIterations,
    WhenFieldNotBoolean,
}

impl InvalidDelegateKind {
    pub fn field_path(self) -> &'static str {
        match self {
            Self::MaxIterations => "completion.delegate.max_iterations",
            Self::WhenFieldNotBoolean => "completion.delegate.when",
            Self::ChildWithoutArtifact => "completion.delegate.child",
            Self::UnsupportedNodeKind | Self::MissingArtifactContract => "completion.delegate",
        }
    }
}

#[derive(Debug)]
pub enum ValidationError {
    InvalidDelegate {
        node: String,
        kind: InvalidDelegateKind,
        reason: String,
    },
    EmptyName,
    InvalidChars {
        name: String,
    },
    EmptyNodes,
    /// nodes に root node（`main` 規約）が定義されていない。
    MissingEntryNode {
        entry: String,
    },
    /// node 名が予約語（kind 名・フィールド名）を使用している。
    ReservedNodeName {
        name: String,
    },
    DuplicateNode {
        name: String,
    },
    MissingFacet {
        node: String,
    },
    /// 合成子の children が空。
    EmptyChildren {
        node: String,
    },
    /// children エントリが存在しないカタログ node を参照している。
    UnknownChildNode {
        node: String,
        child: String,
    },
    /// 同一合成子の children が同じカタログ node を複数回参照している。
    DuplicateChildReference {
        node: String,
        child: String,
    },
    /// fanout の children エントリに rules が書かれている（fanout に辺は無い）。
    RulesOnFanoutChildEntry {
        node: String,
        child: String,
    },
    /// sequence の entry が children のエントリ名を指していない。
    SequenceEntryNotChild {
        node: String,
        entry: String,
    },
    /// sequence の Artifact は engine が組み立てるため宣言できない。
    SequenceArtifactDeclaration {
        node: String,
    },
    /// fanout.items の Artifact field 参照が解決できない。
    InvalidFanoutItemsReference {
        node: String,
        reference: String,
        reason: String,
    },
    /// fanout.items とそれを受ける子パラメータの束縛・型が一致しない。
    FanoutInputMismatch {
        node: String,
        child: String,
        reason: String,
    },
    /// 合成子の子参照の一意性・root 参照禁止などの構造制約違反。
    ChildReferenceViolation {
        node: String,
        child: String,
        reason: String,
    },
    /// children エントリの inputs 配線が不正。
    InvalidInputWiring(Box<InputWiringViolation>),
    /// input パラメータ名が予約供給元名（request / items）を使用している。
    ReservedInputParameterName {
        node: String,
        parameter: String,
    },
    /// 合成子の包含循環（children を辿ると自分自身へ戻る）。
    CompositeInclusionCycle {
        node: String,
        cycle: String,
    },
    /// rules の遷移先が存在しない node を参照
    UnknownRuleTarget {
        node: String,
        target: String,
    },
    /// rules の順序非依存性・網羅性・型付き参照・loop guard が不正
    InvalidRules {
        node: String,
        kind: InvalidRuleKind,
        reason: String,
    },
    /// entry node から到達できない node
    UnreachableNode {
        node: String,
    },
    /// command 種別 node の `command` が空文字
    EmptyCommand {
        node: String,
    },
    /// `nodes` の総数が DoS 防御の上限を超えた
    TooManyNodes {
        count: usize,
        max: usize,
    },
    /// fanout children の数が DoS 防御の上限を超えた
    TooManyFanoutChildren {
        node: String,
        count: usize,
        max: usize,
    },
    /// `artifact` / `input` が存在しない `schemas:` Contract を参照している。
    UnknownSchemaRef {
        node: String,
        slot: &'static str,
        key: String,
    },
    /// `artifact` / `input` の Contract 参照名が安全な identifier ではない。
    InvalidSchemaRef {
        node: String,
        slot: &'static str,
        key: String,
        reason: String,
    },
    /// `schemas:` 内の宣言が JSON Schema subset として矛盾している。
    InvalidSchema {
        schema: String,
        kind: InvalidSchemaKind,
        reason: String,
    },
    /// `artifact:` が Object 以外の Contract を参照している。
    InvalidArtifactSchema {
        node: String,
        contract: String,
    },
    /// command node の `artifact:` Contract が予約 field を宣言している。
    ReservedArtifactField {
        node: String,
        contract: String,
        field: String,
    },
    /// 本文（command / facet）の `{{ ... }}` が解決できない参照を含む。
    InvalidArtifactReference {
        reference: String,
        kind: InvalidArtifactReferenceKind,
        reason: String,
    },
    /// command env が宣言済み input パラメータへ解決できない。
    InvalidEnvironmentReference {
        node: String,
        variable: String,
        reference: String,
        kind: InvalidEnvironmentReferenceKind,
        reason: String,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDelegate { node, kind, reason } => {
                let field = kind.field_path();
                write!(f, "node '{node}' {field}: {reason}")
            }
            Self::EmptyName => write!(f, "ワークフロー名が空です"),
            Self::InvalidChars { name } => write!(
                f,
                "ワークフロー名 '{name}' は先頭を英数字にし、2文字目以降は英数字・ハイフン・アンダースコアのみ使用できます"
            ),
            Self::EmptyNodes => write!(f, "ワークフローにnodeが定義されていません"),
            Self::MissingEntryNode { entry } => {
                write!(f, "nodes must define the root node '{entry}'")
            }
            Self::ReservedNodeName { name } => {
                write!(f, "node name '{name}' is a reserved word and cannot be used")
            }
            Self::DuplicateNode { name } => {
                write!(f, "node名 '{name}' が重複しています")
            }
            Self::MissingFacet { node } => {
                write!(f, "node '{node}' にはファセット参照が必要です")
            }
            Self::EmptyChildren { node } => {
                write!(f, "composite node '{node}' must declare at least one child")
            }
            Self::UnknownChildNode { node, child } => {
                write!(f, "node '{node}' references unknown child node '{child}'")
            }
            Self::DuplicateChildReference { node, child } => {
                write!(
                    f,
                    "composite node '{node}' references child node '{child}' more than once"
                )
            }
            Self::RulesOnFanoutChildEntry { node, child } => {
                write!(
                    f,
                    "fanout node '{node}' child '{child}' cannot declare rules: fanout children have no edges"
                )
            }
            Self::SequenceEntryNotChild { node, entry } => {
                write!(
                    f,
                    "sequence node '{node}' entry '{entry}' must reference one of its children"
                )
            }
            Self::SequenceArtifactDeclaration { node } => {
                write!(
                    f,
                    "sequence node '{node}' cannot declare an artifact: the engine merges child Artifacts"
                )
            }
            Self::InvalidFanoutItemsReference {
                node,
                reference,
                reason,
            } => {
                write!(
                    f,
                    "fanout node '{node}' has invalid items reference '{reference}': {reason}"
                )
            }
            Self::FanoutInputMismatch {
                node,
                child,
                reason,
            } => {
                write!(
                    f,
                    "fanout node '{node}' items do not match child '{child}' input: {reason}"
                )
            }
            Self::ChildReferenceViolation {
                node,
                child,
                reason,
            } => {
                write!(
                    f,
                    "node '{node}' child '{child}' violates child reference constraints: {reason}"
                )
            }
            Self::InvalidInputWiring(violation) => {
                write!(
                    f,
                    "composite node '{}' child '{}' inputs '{}: {}' is invalid: {}",
                    violation.node,
                    violation.child,
                    violation.parameter,
                    violation.source,
                    violation.reason
                )
            }
            Self::ReservedInputParameterName { node, parameter } => {
                write!(
                    f,
                    "node '{node}' input parameter '{parameter}' is a reserved source name and cannot be declared"
                )
            }
            Self::CompositeInclusionCycle { node, cycle } => {
                write!(
                    f,
                    "node '{node}' contains itself through its children ({cycle})"
                )
            }
            Self::UnknownRuleTarget { node, target } => write!(
                f,
                "node '{node}' のrulesが存在しないnode '{target}' を参照しています"
            ),
            Self::InvalidRules { node, reason, .. } => {
                write!(f, "node '{node}' のrulesが不正です: {reason}")
            }
            Self::UnreachableNode { node } => {
                write!(f, "node '{node}' is unreachable from the workflow entrypoint")
            }
            Self::TooManyNodes { count, max } => write!(
                f,
                "node 数 {count} がワークフローあたりの上限 {max} を超えています"
            ),
            Self::TooManyFanoutChildren { node, count, max } => write!(
                f,
                "fanout node '{node}' の child 数 {count} が上限 {max} を超えています"
            ),
            Self::EmptyCommand { node } => {
                write!(
                    f,
                    "command node '{node}' の command は空にできません"
                )
            }
            Self::UnknownSchemaRef { node, slot, key } => {
                write!(
                    f,
                    "node '{node}' の {slot} が存在しない schemas Contract '{key}' を参照しています"
                )
            }
            Self::InvalidSchemaRef {
                node,
                slot,
                key,
                reason,
            } => {
                write!(
                    f,
                    "node '{node}' の {slot} Contract 参照 '{key}' が不正です: {reason}"
                )
            }
            Self::InvalidSchema { schema, reason, .. } => {
                write!(f, "schemas.{schema} の宣言が不正です: {reason}")
            }
            Self::InvalidArtifactSchema { node, contract } => {
                write!(
                    f,
                    "node '{node}' の artifact '{contract}' は Object Contract である必要があります"
                )
            }
            Self::ReservedArtifactField {
                node,
                contract,
                field,
            } => {
                write!(
                    f,
                    "node '{node}' の artifact '{contract}' が予約 field '{field}' を宣言しています"
                )
            }
            Self::InvalidArtifactReference {
                reference, reason, ..
            } => {
                write!(f, "Artifact参照 '{reference}' が不正です: {reason}")
            }
            Self::InvalidEnvironmentReference {
                node,
                variable,
                reference,
                reason,
                ..
            } => write!(
                f,
                "command node '{node}' env '{variable}: {reference}' is invalid: {reason}"
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

fn reference_error_to_validation_error(error: reference::ReferenceResolveError) -> ValidationError {
    match error {
        reference::ReferenceResolveError::ReservedNodeName { name } => {
            ValidationError::InvalidArtifactReference {
                reference: name,
                kind: InvalidArtifactReferenceKind::ReservedArtifactName,
                reason: "`request` is a reserved source name and cannot be a node name".to_string(),
            }
        }
        reference::ReferenceResolveError::UnknownParameter { name } => {
            ValidationError::InvalidArtifactReference {
                reference: name,
                kind: InvalidArtifactReferenceKind::UnknownParameter,
                reason: "`{{ ... }}` references must be declared input parameter names".to_string(),
            }
        }
        reference::ReferenceResolveError::UnknownField { reference, field } => {
            ValidationError::InvalidArtifactReference {
                reference: format!("{reference}.{field}"),
                kind: InvalidArtifactReferenceKind::UnknownField,
                reason: "unknown field on the parameter Contract".to_string(),
            }
        }
        reference::ReferenceResolveError::InvalidInputRef { value } => {
            ValidationError::InvalidArtifactReference {
                reference: value,
                kind: InvalidArtifactReferenceKind::InvalidInputRef,
                reason: "`{{ ... }}` references must be `<parameter>` or `<parameter>.<field>...`"
                    .to_string(),
            }
        }
    }
}

fn environment_reference_error_to_validation_error(
    error: reference::CommandEnvironmentReferenceError,
) -> ValidationError {
    let (kind, reason) = match error.source {
        reference::ReferenceResolveError::UnknownParameter { .. } => (
            InvalidEnvironmentReferenceKind::UnknownParameter,
            "env references must name an input parameter declared by the same command node"
                .to_string(),
        ),
        reference::ReferenceResolveError::UnknownField { .. } => (
            InvalidEnvironmentReferenceKind::UnknownField,
            "unknown field on the input parameter Contract".to_string(),
        ),
        reference::ReferenceResolveError::InvalidInputRef { .. }
        | reference::ReferenceResolveError::ReservedNodeName { .. } => (
            InvalidEnvironmentReferenceKind::InvalidInputRef,
            "env references must be `<parameter>` or `<parameter>.<field>...`".to_string(),
        ),
    };
    ValidationError::InvalidEnvironmentReference {
        node: error.node,
        variable: error.variable,
        reference: error.reference,
        kind,
        reason,
    }
}

fn routing_error_to_validation_error(error: routing::RoutingValidationError) -> ValidationError {
    match error {
        routing::RoutingValidationError::UnknownChildReference { composite, child } => {
            ValidationError::UnknownChildNode {
                node: composite,
                child,
            }
        }
        routing::RoutingValidationError::DuplicateChildReference { composite, child } => {
            ValidationError::DuplicateChildReference {
                node: composite,
                child,
            }
        }
        routing::RoutingValidationError::EmptyChildren { composite } => {
            ValidationError::EmptyChildren { node: composite }
        }
        routing::RoutingValidationError::SequenceEntryNotChild { sequence, entry } => {
            ValidationError::SequenceEntryNotChild {
                node: sequence,
                entry,
            }
        }
        routing::RoutingValidationError::RulesOnFanoutChildEntry { fanout, child } => {
            ValidationError::RulesOnFanoutChildEntry {
                node: fanout,
                child,
            }
        }
        routing::RoutingValidationError::ChildReferenceViolation {
            composite,
            child,
            reason,
        } => ValidationError::ChildReferenceViolation {
            node: composite,
            child,
            reason,
        },
        routing::RoutingValidationError::UnknownRuleTarget { node, target } => {
            ValidationError::UnknownRuleTarget { node, target }
        }
        routing::RoutingValidationError::MultipleDiscriminators { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::MultipleDiscriminators,
                reason: "rules can contain at most one when or switch discriminator".to_string(),
            }
        }
        routing::RoutingValidationError::MultipleLoopGuards { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::MultipleLoopGuards,
                reason: "rules can contain at most one loop_guard".to_string(),
            }
        }
        routing::RoutingValidationError::MultipleNextCatchAll { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::MultipleNextCatchAll,
                reason: "rules can contain at most one next catch-all".to_string(),
            }
        }
        routing::RoutingValidationError::StandaloneNextWithDiscriminator { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::StandaloneNextWithDiscriminator,
                reason: "standalone next cannot be combined with when or switch".to_string(),
            }
        }
        routing::RoutingValidationError::WhenFieldNotBoolean {
            node,
            field,
            reason,
        } => ValidationError::InvalidRules {
            node,
            kind: InvalidRuleKind::WhenFieldNotBoolean,
            reason: reason
                .unwrap_or_else(|| format!("when.on field '{field}' must be a required boolean")),
        },
        routing::RoutingValidationError::SwitchFieldNotEnum {
            node,
            field,
            reason,
        } => ValidationError::InvalidRules {
            node,
            kind: InvalidRuleKind::SwitchFieldNotEnum,
            reason: reason
                .unwrap_or_else(|| format!("switch.on field '{field}' must be a required enum")),
        },
        routing::RoutingValidationError::SwitchUnknownCase { node, field, case } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::SwitchUnknownCase,
                reason: format!("switch case '{case}' is not declared in enum field '{field}'"),
            }
        }
        routing::RoutingValidationError::SwitchMissingCases {
            node,
            field,
            missing,
        } => ValidationError::InvalidRules {
            node,
            kind: InvalidRuleKind::SwitchMissingCases,
            reason: format!(
                "switch on '{field}' is missing enum cases [{}] and requires next",
                missing.join(", ")
            ),
        },
        routing::RoutingValidationError::SwitchExhaustiveHasNext { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::SwitchExhaustiveHasNext,
                reason: "exhaustive switch cannot also define next catch-all".to_string(),
            }
        }
        routing::RoutingValidationError::SwitchRequiresNext { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::SwitchRequiresNext,
                reason: "command artifact routing on Contract field requires next catch-all"
                    .to_string(),
            }
        }
        routing::RoutingValidationError::DiscriminatorWithoutArtifact { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::DiscriminatorWithoutArtifact,
                reason: "nodes without an artifact cannot be routed with when or switch rules"
                    .to_string(),
            }
        }
        routing::RoutingValidationError::LoopGuardMaxIterations { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::LoopGuardMaxIterations,
                reason: "loop_guard.max_iterations must be greater than 0".to_string(),
            }
        }
        routing::RoutingValidationError::CycleWithoutLoopGuard { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::CycleWithoutLoopGuard,
                reason: "cycle reachable from this node has no loop_guard on cycle nodes"
                    .to_string(),
            }
        }
        routing::RoutingValidationError::UnreachableNode { node } => {
            ValidationError::UnreachableNode { node }
        }
    }
}

pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    match WorkflowDefinitionName::new(name) {
        Ok(_) => Ok(()),
        Err(_) if name.is_empty() => Err(ValidationError::EmptyName),
        Err(_) => Err(ValidationError::InvalidChars {
            name: name.to_string(),
        }),
    }
}

pub fn validate_workflow_shape(workflow: &WorkflowDefinition) -> Result<(), WorkflowError> {
    if workflow.nodes.is_empty() {
        return Err(WorkflowError::validation("workflow has no nodes"));
    }
    Ok(())
}

/// インライン宣言・無名エントリは load 時の正規化で `nodes` カタログへ登録される
/// ため、カタログ長がそのまま再帰的な総 node 数になる。
fn total_node_count(workflow: &WorkflowDefinition) -> usize {
    workflow.nodes.len()
}

/// fanout の items とそれを受ける子パラメータの束縛・型を検証する。
///
/// items が供給されるパラメータは、entry の inputs での明示配線
/// （`<パラメータ>: items`）か、子のパラメータがちょうど1つの場合の自動束縛で
/// 決まる。型あり（Contract 付き）パラメータには要素 Contract の一致を要求する。
fn collect_fanout_items_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for parent in &workflow.nodes {
        let Some(fanout) = parent.fanout() else {
            continue;
        };
        let Some(items) = &fanout.items else {
            continue;
        };

        enum ElementShape<'a> {
            Contract(String),
            Literals(&'a [serde_json::Value]),
        }

        let element_shape = match items {
            ItemsSource::Literal(values) => ElementShape::Literals(values),
            ItemsSource::ArtifactField { node, field_path } => {
                let reference_value = format!("{node}.{}", field_path.as_string());
                match reference::artifact_field_schema(workflow, node, field_path) {
                    Err(reason) => {
                        errors.push(ValidationError::InvalidFanoutItemsReference {
                            node: parent.name.clone(),
                            reference: reference_value.clone(),
                            reason,
                        });
                        continue;
                    }
                    Ok(reference::ResolvedNodeField::Leaf {
                        schema:
                            SchemaDef::Array {
                                items: element_contract,
                            },
                        ..
                    }) => ElementShape::Contract(element_contract),
                    Ok(_) => {
                        errors.push(ValidationError::InvalidFanoutItemsReference {
                            node: parent.name.clone(),
                            reference: reference_value,
                            reason: "items reference must resolve to an array field".to_string(),
                        });
                        continue;
                    }
                }
            }
        };

        for entry in &fanout.children {
            let Some(child) = workflow.node_by_name(&entry.name) else {
                continue;
            };
            if child.is_composite() {
                // ネスト合成子は W3 未対応として別途報告する。
                continue;
            }

            let explicit_receivers: Vec<&str> = entry
                .inputs
                .iter()
                .filter(|(_, source)| source.root() == reference::ITEMS_SOURCE)
                .map(|(parameter, _)| parameter.as_str())
                .collect();

            let receivers: Vec<&InputParam> = if !explicit_receivers.is_empty() {
                explicit_receivers
                    .iter()
                    .filter_map(|name| child.input_parameter(name))
                    .collect()
            } else {
                let auto_receiver = match child.input.as_slice() {
                    [sole]
                        if !entry
                            .inputs
                            .iter()
                            .any(|(parameter, _)| parameter == &sole.name) =>
                    {
                        Some(sole)
                    }
                    _ => None,
                };
                match auto_receiver {
                    Some(receiver) => vec![receiver],
                    None => {
                        errors.push(ValidationError::FanoutInputMismatch {
                            node: parent.name.clone(),
                            child: entry.name.clone(),
                            reason: "fanout supplies items but no child parameter receives them"
                                .to_string(),
                        });
                        continue;
                    }
                }
            };

            for receiver in receivers {
                let Some(receiver_contract) = receiver.contract.as_deref() else {
                    continue;
                };
                match &element_shape {
                    ElementShape::Contract(element_contract) => {
                        if receiver_contract != element_contract {
                            errors.push(ValidationError::FanoutInputMismatch {
                                node: parent.name.clone(),
                                child: entry.name.clone(),
                                reason: format!(
                                    "items element Contract '{element_contract}' does not match parameter '{}' Contract '{receiver_contract}'",
                                    receiver.name
                                ),
                            });
                        }
                    }
                    ElementShape::Literals(values) => {
                        let Some(schema) = workflow.schemas.get(receiver_contract) else {
                            continue;
                        };
                        if let Some((item_index, violations)) =
                            values.iter().enumerate().find_map(|(item_index, value)| {
                                contract_schema::validate(value, schema, &workflow.schemas)
                                    .err()
                                    .map(|violations| (item_index, violations))
                            })
                        {
                            let reason = violations
                                .first()
                                .map(|violation| {
                                    format!("{}: {}", violation.path, violation.reason)
                                })
                                .unwrap_or_else(|| "Contract validation failed".to_string());
                            errors.push(ValidationError::FanoutInputMismatch {
                                node: parent.name.clone(),
                                child: entry.name.clone(),
                                reason: format!(
                                    "literal item at index {item_index} does not match Contract '{receiver_contract}': {reason}"
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    errors
}

fn collect_delegate_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for node in &workflow.nodes {
        let Some(delegate) = &node.completion.delegate else {
            continue;
        };
        let invalid = |kind, reason: &str| ValidationError::InvalidDelegate {
            node: node.name.clone(),
            kind,
            reason: reason.to_string(),
        };
        if !node.is_session() {
            errors.push(invalid(
                InvalidDelegateKind::UnsupportedNodeKind,
                "delegate can only be declared by a Session",
            ));
        } else if node.artifact.is_none() {
            errors.push(invalid(
                InvalidDelegateKind::MissingArtifactContract,
                "delegate requires an artifact Contract",
            ));
        }
        if delegate.max_iterations == 0 {
            errors.push(invalid(
                InvalidDelegateKind::MaxIterations,
                "delegate max_iterations must be at least 1",
            ));
        }
        match workflow.node_by_name(&delegate.child) {
            None => errors.push(ValidationError::UnknownChildNode {
                node: node.name.clone(),
                child: delegate.child.clone(),
            }),
            Some(child) if !reference::node_has_artifact(child) => {
                errors.push(invalid(
                    InvalidDelegateKind::ChildWithoutArtifact,
                    "delegate child must produce an Artifact",
                ));
            }
            _ => {}
        }
        if let Some(contract) = node.artifact.as_deref() {
            if let Some(SchemaDef::Object { properties, .. }) = workflow.schemas.get(contract) {
                if properties.contains_key("child") {
                    errors.push(ValidationError::ReservedArtifactField {
                        node: node.name.clone(),
                        contract: contract.to_string(),
                        field: "child".into(),
                    });
                }
            }
        }
        errors.extend(delegate.when.validate(&mut |field| {
            routing::validate_artifact_field(
                workflow,
                node,
                field,
                contract_schema::RoutingFieldKind::Boolean,
                InvalidDelegateKind::WhenFieldNotBoolean.field_path(),
            )
            .map(|_| ())
            .map_err(|reason| ValidationError::InvalidDelegate {
                node: node.name.clone(),
                kind: InvalidDelegateKind::WhenFieldNotBoolean,
                reason,
            })
        }));
    }
    errors
}

/// 全合成子スコープの children エントリの inputs 配線を検証する。
///
/// sequence の供給元解決は自分の children（兄弟）+ 自合成子の input パラメータ +
/// 予約供給元 `request` に閉じる。fanout の子は並走し兄弟を持たないため、
/// 供給元は自 fanout の input パラメータ + `request` + `items` に閉じる
/// （外部文脈は親が fanout のパラメータへ配線して渡す）。
fn collect_children_wiring_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    for owner in &workflow.nodes {
        let delegate_entry = owner
            .completion
            .delegate
            .as_ref()
            .map(|delegate| vec![delegate.child_entry()]);
        let (children, fanout_has_items) = match &owner.kind {
            NodeKind::Sequence(sequence) => (&sequence.children, None),
            NodeKind::Fanout(fanout) => (&fanout.children, Some(fanout.items.is_some())),
            NodeKind::Session(_) if delegate_entry.is_some() => {
                (delegate_entry.as_ref().unwrap(), None)
            }
            _ => continue,
        };
        let is_fanout_scope = fanout_has_items.is_some();
        let sibling_names: BTreeSet<&str> = if delegate_entry.is_some() {
            std::iter::once(owner.name.as_str())
                .filter(|name| {
                    !crate::domain::workflow::value_objects::NodeNamespace::is_synthesized(name)
                })
                .collect()
        } else {
            children.iter().map(|entry| entry.name.as_str()).collect()
        };
        let own_params: BTreeSet<&str> = owner.input_parameter_names().collect();

        for entry in children {
            let child = workflow.node_by_name(&entry.name);
            for (parameter, source) in &entry.inputs {
                let push =
                    |errors: &mut Vec<ValidationError>, kind: InputWiringKind, reason: String| {
                        errors.push(ValidationError::InvalidInputWiring(Box::new(
                            InputWiringViolation {
                                node: owner.name.clone(),
                                child: entry.name.clone(),
                                parameter: parameter.clone(),
                                source: source.raw().to_string(),
                                kind,
                                reason,
                            },
                        )));
                    };

                if let Some(child) = child {
                    if child.input_parameter(parameter).is_none() {
                        push(
                            &mut errors,
                            InputWiringKind::UnknownParameter,
                            format!(
                                "child '{}' does not declare input parameter '{parameter}'",
                                entry.name
                            ),
                        );
                    }
                }

                let root = source.root();
                let Some(field) = source.field_path() else {
                    push(
                        &mut errors,
                        InputWiringKind::InvalidSourceFormat,
                        "source must be `<name>` or `<name>.<field>...`".to_string(),
                    );
                    continue;
                };

                if root == reference::REQUEST_ARTIFACT {
                    if !field.is_empty() {
                        push(
                            &mut errors,
                            InputWiringKind::InvalidSourceFormat,
                            "`request` has no fields".to_string(),
                        );
                    }
                    continue;
                }
                if root == reference::ITEMS_SOURCE {
                    match fanout_has_items {
                        Some(true) => {
                            if !field.is_empty() {
                                push(
                                    &mut errors,
                                    InputWiringKind::InvalidSourceFormat,
                                    "`items` has no fields".to_string(),
                                );
                            }
                        }
                        Some(false) => push(
                            &mut errors,
                            InputWiringKind::ItemsUnavailable,
                            "fanout does not declare items".to_string(),
                        ),
                        None => push(
                            &mut errors,
                            InputWiringKind::ItemsUnavailable,
                            "`items` is only available inside a fanout with items".to_string(),
                        ),
                    }
                    continue;
                }

                // sequence は兄弟 + 自パラメータ、fanout は自パラメータのみ
                // （fanout の子に兄弟参照は無い）。
                let is_node_source = !is_fanout_scope
                    && root != entry.name
                    && (sibling_names.contains(root)
                        || (delegate_entry.is_some()
                            && source.is_synthesized_node_artifact()
                            && root == owner.name));
                let is_own_param = own_params.contains(root);
                match (is_node_source, is_own_param) {
                    (true, true) => push(
                        &mut errors,
                        InputWiringKind::AmbiguousSource,
                        format!(
                            "'{root}' matches both a sibling node and an input parameter of '{}'",
                            owner.name
                        ),
                    ),
                    (true, false) => {
                        let Some(source_node) = workflow.node_by_name(root) else {
                            // 兄弟エントリの参照先不明は UnknownChildNode 側で報告する。
                            continue;
                        };
                        if !reference::node_has_artifact(source_node) {
                            push(
                                &mut errors,
                                InputWiringKind::UnavailableSourceArtifact,
                                format!("source node '{root}' does not produce an Artifact"),
                            );
                        } else if !field.is_empty() {
                            if let Err(reason) =
                                validate_node_source_field_path(workflow, source_node, &field)
                            {
                                push(&mut errors, InputWiringKind::UnknownSourceField, reason);
                            }
                        }
                    }
                    (false, true) => {
                        if let Some(param) = owner.input_parameter(root) {
                            if field.is_empty() {
                                continue;
                            }
                            if let Some(contract) = param.contract.as_deref() {
                                let resolution = workflow
                                    .schemas
                                    .get(contract)
                                    .ok_or_else(|| {
                                        format!(
                                        "parameter '{root}' Contract '{contract}' is not declared"
                                    )
                                    })
                                    .and_then(|schema| {
                                        contract_schema::resolve_field_path(schema, &field)
                                            .map(|_| ())
                                            .map_err(|error| {
                                                field_path_resolution_reason(
                                                    &format!(
                                                        "parameter '{root}' Contract '{contract}'"
                                                    ),
                                                    &error,
                                                )
                                            })
                                    });
                                if let Err(reason) = resolution {
                                    push(&mut errors, InputWiringKind::UnknownSourceField, reason);
                                }
                            }
                        }
                    }
                    (false, false) => {
                        let reason = if is_fanout_scope {
                            format!(
                                "'{root}' is not an input parameter of fanout '{}', `request`, or `items` (fanout children cannot reference nodes directly; wire outer values through the fanout's input parameters)",
                                owner.name
                            )
                        } else {
                            format!(
                                "'{root}' is not a sibling node, an input parameter of '{}', or `request`",
                                owner.name
                            )
                        };
                        push(&mut errors, InputWiringKind::UnknownSource, reason)
                    }
                }
            }
        }
    }

    errors
}

fn validate_node_source_field_path(
    workflow: &WorkflowDefinition,
    node: &NodeDefinition,
    field_path: &FieldPath,
) -> Result<(), String> {
    reference::resolve_node_field_path(workflow, node, field_path)
        .map(|_| ())
        .map_err(|error| match error {
            reference::NodeFieldPathError::ArtifactNotObject => format!(
                "source node '{}' Artifact Contract is not an object",
                node.name
            ),
            reference::NodeFieldPathError::NoReferenceableArtifact => format!(
                "source node '{}' Artifact has no field path '{field_path}'",
                node.name
            ),
            reference::NodeFieldPathError::Segment(error) => field_path_resolution_reason(
                &format!("source node '{}' Artifact", node.name),
                &error,
            ),
        })
}

fn field_path_resolution_reason(
    source: &str,
    error: &contract_schema::FieldPathResolutionError,
) -> String {
    match error.kind {
        contract_schema::FieldPathResolutionErrorKind::NonObject => format!(
            "{source} cannot resolve segment {} ('{}') from a non-object value",
            error.position + 1,
            error.segment
        ),
        contract_schema::FieldPathResolutionErrorKind::MissingProperty => format!(
            "{source} does not declare segment {} ('{}')",
            error.position + 1,
            error.segment
        ),
    }
}

/// 合成子の包含循環（children に置かれた合成子を辿ると自分自身へ戻る）を検出する。
/// 深さの総量は `MAX_NODES_PER_WORKFLOW` が縛るため、循環だけが非有界の芽になる。
fn collect_inclusion_cycle_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let composite_children: BTreeMap<&str, Vec<&str>> = workflow
        .nodes
        .iter()
        .filter_map(|node| {
            let children = match &node.kind {
                NodeKind::Sequence(sequence) => &sequence.children,
                NodeKind::Fanout(fanout) => &fanout.children,
                _ => {
                    return node
                        .completion
                        .delegate
                        .as_ref()
                        .map(|delegate| (node.name.as_str(), vec![delegate.child.as_str()]));
                }
            };
            Some((
                node.name.as_str(),
                children
                    .iter()
                    .filter(|entry| {
                        workflow
                            .node_by_name(&entry.name)
                            .is_some_and(|node| node.has_child_executions())
                    })
                    .map(|entry| entry.name.as_str())
                    .collect(),
            ))
        })
        .collect();
    composite_children
        .keys()
        .filter_map(|start| {
            inclusion_cycle_from(start, &composite_children).map(|cycle| {
                ValidationError::CompositeInclusionCycle {
                    node: (*start).to_string(),
                    cycle: cycle.join(" -> "),
                }
            })
        })
        .collect()
}

/// `start` から包含グラフを辿って `start` 自身へ戻る最初のパスを返す。
fn inclusion_cycle_from<'a>(
    start: &'a str,
    graph: &BTreeMap<&'a str, Vec<&'a str>>,
) -> Option<Vec<&'a str>> {
    fn dfs<'a>(
        current: &'a str,
        start: &'a str,
        graph: &BTreeMap<&'a str, Vec<&'a str>>,
        path: &mut Vec<&'a str>,
        visited: &mut BTreeSet<&'a str>,
    ) -> bool {
        for next in graph.get(current).into_iter().flatten() {
            if *next == start {
                path.push(next);
                return true;
            }
            if visited.insert(next) {
                path.push(next);
                if dfs(next, start, graph, path, visited) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
    let mut path = vec![start];
    let mut visited = BTreeSet::new();
    dfs(start, start, graph, &mut path, &mut visited).then_some(path)
}

/// sequence の artifact 宣言を拒否する。
fn collect_sequence_artifact_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    workflow
        .nodes
        .iter()
        .filter(|node| node.artifact.is_some() && node.sequence().is_some())
        .map(|node| ValidationError::SequenceArtifactDeclaration {
            node: node.name.clone(),
        })
        .collect()
}

fn collect_reserved_parameter_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for node in &workflow.nodes {
        for parameter in node.input_parameter_names() {
            if parameter == reference::REQUEST_ARTIFACT || parameter == reference::ITEMS_SOURCE {
                errors.push(ValidationError::ReservedInputParameterName {
                    node: node.name.clone(),
                    parameter: parameter.to_string(),
                });
            }
        }
    }
    errors
}

pub fn validate(workflow: &WorkflowDefinition) -> Result<(), ValidationError> {
    validate_name(&workflow.name)?;

    if workflow.nodes.is_empty() {
        return Err(ValidationError::EmptyNodes);
    }
    if workflow.entry_node().is_none() {
        return Err(ValidationError::MissingEntryNode {
            entry: workflow.entry.clone(),
        });
    }
    if let Some(node) = workflow
        .nodes
        .iter()
        .find(|node| is_reserved_node_name(&node.name))
    {
        return Err(ValidationError::ReservedNodeName {
            name: node.name.clone(),
        });
    }
    if let Some(err) = collect_node_count_errors(workflow).into_iter().next() {
        return Err(err);
    }
    validate_schema_refs(workflow)?;
    if let Some(err) = reference::validate_workflow_reference_diagnostics(workflow)
        .into_iter()
        .next()
    {
        return Err(reference_error_to_validation_error(err));
    }
    if let Some(err) = reference::validate_workflow_command_environment_references(workflow)
        .into_iter()
        .next()
    {
        return Err(environment_reference_error_to_validation_error(err));
    }

    // 重複 node 名を検出する。
    let mut seen_names = HashSet::new();
    for node in &workflow.nodes {
        if !seen_names.insert(node.name.as_str()) {
            return Err(ValidationError::DuplicateNode {
                name: node.name.clone(),
            });
        }
    }
    if let Some(err) = routing::validate_rules(workflow).into_iter().next() {
        return Err(routing_error_to_validation_error(err));
    }
    if let Some(err) = collect_inclusion_cycle_errors(workflow).into_iter().next() {
        return Err(err);
    }
    if let Some(err) = collect_delegate_errors(workflow).into_iter().next() {
        return Err(err);
    }
    if let Some(err) = collect_children_wiring_errors(workflow).into_iter().next() {
        return Err(err);
    }
    if let Some(err) = collect_fanout_items_errors(workflow).into_iter().next() {
        return Err(err);
    }
    if let Some(err) = collect_reserved_parameter_errors(workflow)
        .into_iter()
        .next()
    {
        return Err(err);
    }
    if let Some(err) = collect_sequence_artifact_errors(workflow)
        .into_iter()
        .next()
    {
        return Err(err);
    }

    for node in &workflow.nodes {
        validate_node_kind_fields(node)?;
        if let Some(err) = check_missing_facet(node) {
            return Err(err);
        }
    }

    Ok(())
}

/// `schemas:` 宣言と `artifact` / `input` 参照を検証する。
///
/// validation.rs は facet I/O を持たず、workflow definition に含まれる
/// backend-owned schema state だけを読む。
pub fn validate_schema_refs(workflow: &WorkflowDefinition) -> Result<(), ValidationError> {
    for (name, schema) in &workflow.schemas {
        if name == reference::REQUEST_ARTIFACT {
            return Err(ValidationError::InvalidArtifactReference {
                reference: name.to_string(),
                kind: InvalidArtifactReferenceKind::ReservedArtifactName,
                reason: "`request` is a reserved Artifact name and cannot be declared in schemas"
                    .to_string(),
            });
        }
        if !contract_schema::is_safe_identifier(name) {
            return Err(ValidationError::InvalidSchema {
                schema: name.to_string(),
                kind: InvalidSchemaKind::InvalidDeclaration,
                reason: safe_identifier_message().to_string(),
            });
        }
        validate_schema_def(name, schema, workflow)?;
    }

    for node in &workflow.nodes {
        validate_node_schema_refs(
            &node.name,
            node.artifact.as_deref(),
            &node.input,
            node.is_command(),
            node.is_fanout(),
            workflow,
        )?;
    }

    Ok(())
}

/// facet 本文などの `{{ ... }}` を、使用 node の input パラメータ宣言と突合する。
pub fn validate_template_references_for_node(
    workflow: &WorkflowDefinition,
    node: &NodeDefinition,
    content: &str,
) -> Vec<ValidationError> {
    reference::validate_template_references_for_node(node, &workflow.schemas, content)
        .into_iter()
        .map(reference_error_to_validation_error)
        .collect()
}

fn validate_schema_def(
    name: &str,
    schema: &SchemaDef,
    workflow: &WorkflowDefinition,
) -> Result<(), ValidationError> {
    match schema {
        SchemaDef::Object {
            properties,
            required,
            ..
        } => {
            for field in required {
                if !properties.contains_key(field) {
                    return Err(ValidationError::InvalidSchema {
                        schema: name.to_string(),
                        kind: InvalidSchemaKind::InvalidDeclaration,
                        reason: format!("required field '{field}' is not declared in properties"),
                    });
                }
            }
            for (field, property_schema) in properties {
                validate_schema_def(
                    &format!("{name}.properties.{field}"),
                    property_schema,
                    workflow,
                )?;
            }
        }
        SchemaDef::Array { items } => {
            if !contract_schema::is_safe_identifier(items) {
                return Err(ValidationError::InvalidSchema {
                    schema: name.to_string(),
                    kind: InvalidSchemaKind::InvalidDeclaration,
                    reason: safe_identifier_message().to_string(),
                });
            }
            if !workflow.schemas.contains_key(items) {
                return Err(ValidationError::InvalidSchema {
                    schema: name.to_string(),
                    kind: InvalidSchemaKind::UnknownSchemaReference,
                    reason: format!("array.items references unknown schemas '{items}'"),
                });
            }
        }
        SchemaDef::String { r#enum } => {
            if r#enum.as_ref().is_some_and(Vec::is_empty) {
                return Err(ValidationError::InvalidSchema {
                    schema: name.to_string(),
                    kind: InvalidSchemaKind::InvalidDeclaration,
                    reason: "enum must contain at least one value".to_string(),
                });
            }
        }
        SchemaDef::Boolean | SchemaDef::Integer | SchemaDef::Number => {}
    }
    Ok(())
}

fn validate_node_schema_refs(
    node_name: &str,
    artifact: Option<&str>,
    input: &[InputParam],
    is_command: bool,
    is_fanout: bool,
    workflow: &WorkflowDefinition,
) -> Result<(), ValidationError> {
    if let Some(contract) = artifact {
        validate_schema_reference_identifier(node_name, "artifact", contract)?;
        let schema =
            workflow
                .schemas
                .get(contract)
                .ok_or_else(|| ValidationError::UnknownSchemaRef {
                    node: node_name.to_string(),
                    slot: "artifact",
                    key: contract.to_string(),
                })?;
        if is_fanout || !matches!(schema, SchemaDef::Object { .. }) {
            return Err(ValidationError::InvalidArtifactSchema {
                node: node_name.to_string(),
                contract: contract.to_string(),
            });
        }
        if let SchemaDef::Object { properties, .. } = schema {
            if properties.contains_key("worktree") {
                return Err(ValidationError::ReservedArtifactField {
                    node: node_name.to_string(),
                    contract: contract.to_string(),
                    field: "worktree".to_string(),
                });
            }
        }
        if is_command {
            if let Some(field) = contract_schema::schema_declares_command_reserved_field(schema) {
                return Err(ValidationError::ReservedArtifactField {
                    node: node_name.to_string(),
                    contract: contract.to_string(),
                    field,
                });
            }
        }
    }

    for param in input {
        let Some(contract) = param.contract.as_deref() else {
            continue;
        };
        validate_schema_reference_identifier(node_name, "input", contract)?;
        if !workflow.schemas.contains_key(contract) {
            return Err(ValidationError::UnknownSchemaRef {
                node: node_name.to_string(),
                slot: "input",
                key: contract.to_string(),
            });
        }
    }

    Ok(())
}

fn validate_schema_reference_identifier(
    node: &str,
    slot: &'static str,
    key: &str,
) -> Result<(), ValidationError> {
    if contract_schema::is_safe_identifier(key) {
        return Ok(());
    }
    Err(ValidationError::InvalidSchemaRef {
        node: node.to_string(),
        slot,
        key: key.to_string(),
        reason: safe_identifier_message().to_string(),
    })
}

fn safe_identifier_message() -> &'static str {
    "must start with an ASCII alphanumeric character and contain only ASCII alphanumeric characters, '-' or '_'"
}

/// node 数上限 (`MAX_NODES_PER_WORKFLOW`) と fanout children 数上限
/// (`MAX_FANOUT_CHILDREN`) の DoS ガードを評価する。
///
/// `TooManyNodes` を検出した時点で後続の per-node `TooManyFanoutChildren`
/// 検査はスキップし、上限超過 1 件のみを返す（名前空間構築すら無意味な状態のため）。
/// `validate` / `validate_all` の両経路から呼ばれ、呼び出し側はそれぞれ
/// 「最初のエラーで return」「全件 push して以降のチェックを打ち切る」と
/// 消費方法を切り替える。
fn collect_node_count_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let total_nodes = total_node_count(workflow);
    if total_nodes > MAX_NODES_PER_WORKFLOW {
        errors.push(ValidationError::TooManyNodes {
            count: total_nodes,
            max: MAX_NODES_PER_WORKFLOW,
        });
        return errors;
    }
    for node in &workflow.nodes {
        if let Some(fanout) = node.fanout() {
            if fanout.children.len() > MAX_FANOUT_CHILDREN {
                errors.push(ValidationError::TooManyFanoutChildren {
                    node: node.name.clone(),
                    count: fanout.children.len(),
                    max: MAX_FANOUT_CHILDREN,
                });
            }
        }
    }
    errors
}

/// session に facet 参照が無い場合に
/// `MissingFacet` を返す。command node は `command` を持つため facet が
/// 不要であり、この検査の対象外（必須フィールドは `validate_node_kind_fields` で検証済み）。
///
/// `validate` / `validate_all` の両経路で同じ判定を行うため共通化する。
fn check_missing_facet(node: &NodeDefinition) -> Option<ValidationError> {
    if node.is_session() && !node.has_facet_refs() {
        Some(ValidationError::MissingFacet {
            node: node.name.clone(),
        })
    } else {
        None
    }
}

/// node kind ごとの許可フィールドを検証する。
fn validate_node_kind_fields(node: &NodeDefinition) -> Result<(), ValidationError> {
    match node.kind_name() {
        NodeKindName::Command => {
            let command = node
                .command()
                .expect("command node must expose command spec");
            if command.trim().is_empty() {
                return Err(ValidationError::EmptyCommand {
                    node: node.name.clone(),
                });
            }
        }
        NodeKindName::Session | NodeKindName::Fanout | NodeKindName::Sequence => {}
    }
    Ok(())
}
/// 診断用: 全てのバリデーションエラーを収集して返す。
/// `validate` は最初のエラーで早期リターンするが、診断エンジンでは全エラーを網羅的に報告したいため、
/// 構造的に安全な範囲でエラーを蓄積する。
/// 名前空間構築に失敗するレベルのエラー（EmptyName, EmptyNodes, DuplicateNode等）は
/// 後続チェックが信頼できないため、そこで打ち切って返す。
pub fn validate_all(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if let Err(e) = validate_name(&workflow.name) {
        errors.push(e);
        return errors;
    }

    if workflow.nodes.is_empty() {
        errors.push(ValidationError::EmptyNodes);
        return errors;
    }
    if workflow.entry_node().is_none() {
        errors.push(ValidationError::MissingEntryNode {
            entry: workflow.entry.clone(),
        });
        // root 不在では到達可能性以降のチェックが信頼できないため打ち切る。
        return errors;
    }
    for node in &workflow.nodes {
        if is_reserved_node_name(&node.name) {
            errors.push(ValidationError::ReservedNodeName {
                name: node.name.clone(),
            });
        }
    }
    let count_errors = collect_node_count_errors(workflow);
    let has_too_many_nodes = count_errors
        .iter()
        .any(|e| matches!(e, ValidationError::TooManyNodes { .. }));
    errors.extend(count_errors);
    if has_too_many_nodes {
        // node 数上限超過時は名前空間構築自体が無意味なため打ち切る
        // （validate と同じ短絡条件）。
        return errors;
    }
    if let Err(e) = validate_schema_refs(workflow) {
        errors.push(e);
    }
    errors.extend(
        reference::validate_workflow_reference_diagnostics(workflow)
            .into_iter()
            .map(reference_error_to_validation_error),
    );
    errors.extend(
        reference::validate_workflow_command_environment_references(workflow)
            .into_iter()
            .map(environment_reference_error_to_validation_error),
    );

    // 重複 node 名を検出し、あれば蓄積するが、以降のチェックは続行
    let mut seen_names = HashSet::new();
    let mut has_dup = false;
    for node in &workflow.nodes {
        if !seen_names.insert(node.name.as_str()) {
            errors.push(ValidationError::DuplicateNode {
                name: node.name.clone(),
            });
            has_dup = true;
        }
    }
    // 名前重複がある場合、参照チェックが不正確になるため打ち切り
    if has_dup {
        return errors;
    }
    errors.extend(
        routing::validate_rules(workflow)
            .into_iter()
            .map(routing_error_to_validation_error),
    );
    errors.extend(
        routing::validate_reachability(workflow)
            .into_iter()
            .map(routing_error_to_validation_error),
    );
    errors.extend(collect_inclusion_cycle_errors(workflow));
    errors.extend(collect_delegate_errors(workflow));
    errors.extend(collect_children_wiring_errors(workflow));
    errors.extend(collect_fanout_items_errors(workflow));
    errors.extend(collect_reserved_parameter_errors(workflow));
    errors.extend(collect_sequence_artifact_errors(workflow));

    for node in &workflow.nodes {
        if let Err(e) = validate_node_kind_fields(node) {
            errors.push(e);
        }
        if let Some(err) = check_missing_facet(node) {
            errors.push(err);
        }
    }

    errors
}

#[cfg(test)]
#[path = "validation_test.rs"]
mod validation_tests;
