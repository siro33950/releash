use crate::domain::workflow::services::{contract_schema, reference, routing};
use crate::domain::workflow::value_objects::{MAX_FANOUT_CHILDREN, MAX_NODES_PER_WORKFLOW};
use crate::domain::workflow::{
    ItemsSource, NodeDefinition, NodeKindName, SchemaDef, WorkflowDefinition,
    WorkflowDefinitionName, WorkflowError,
};
use std::collections::{HashMap, HashSet};
use std::fmt;

const ALLOWED_PERMISSION_MODES: &str = "ask, edit, full";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSchemaKind {
    InvalidDeclaration,
    UnknownSchemaReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidArtifactReferenceKind {
    ReservedArtifactName,
    UnknownNode,
    UnavailableArtifact,
    UnknownField,
    ItemOutOfScope,
    InvalidInputRef,
    InputsNotAllowedOnFanout,
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
    DiscriminatorOnFanout,
    DiscriminatorWithoutArtifact,
    LoopGuardMaxIterations,
    CycleWithoutLoopGuard,
}

#[derive(Debug)]
pub enum ValidationError {
    EmptyName,
    InvalidChars {
        name: String,
    },
    EmptyNodes,
    DuplicateNode {
        name: String,
    },
    MissingFacet {
        node: String,
    },
    EmptyFanoutChildren {
        node: String,
    },
    /// fanout.child が存在しない top-level node を参照している。
    UnknownFanoutChild {
        node: String,
        child: String,
    },
    /// fanout.items の Artifact field 参照が解決できない。
    InvalidFanoutItemsReference {
        node: String,
        reference: String,
        reason: String,
    },
    /// fanout.items と child.input の型・有無が一致しない。
    FanoutInputMismatch {
        node: String,
        child: String,
        reason: String,
    },
    /// fanout child の entry / 通常遷移 / 入れ子禁止に違反している。
    FanoutChildLeafViolation {
        node: String,
        child: String,
        reason: String,
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
    /// 無効な permission mode が指定されている
    InvalidPermissionMode {
        node: String,
        value: String,
    },
    /// node に permission が指定されていない（必須）
    MissingPermissionMode {
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
    /// fanout.child の数が DoS 防御の上限を超えた
    TooManyFanoutChildren {
        node: String,
        count: usize,
        max: usize,
    },
    /// 存在しないモデルが指定されている
    UnknownModel {
        node: String,
        value: String,
    },
    /// モデルIDが形式として無効（空文字・空白のみ・制御文字・上限長超過など）。
    /// `reason` には `ModelId` の戻り値（理由文言）を保持し、
    /// 呼び出し側・ログで未登録（UnknownModel）と区別できるようにする。
    InvalidModelFormat {
        node: String,
        value: String,
        reason: String,
    },
    /// モデルIDの形式は有効だが、バックエンド所属を一意に解決できない。
    ModelResolutionFailed {
        node: String,
        value: String,
        reason: String,
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
    /// `inputs:` または `{{ ... }}` が解決できない Artifact 参照を含む。
    InvalidArtifactReference {
        reference: String,
        kind: InvalidArtifactReferenceKind,
        reason: String,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "ワークフロー名が空です"),
            Self::InvalidChars { name } => write!(
                f,
                "ワークフロー名 '{name}' は先頭を英数字にし、2文字目以降は英数字・ハイフン・アンダースコアのみ使用できます"
            ),
            Self::EmptyNodes => write!(f, "ワークフローにnodeが定義されていません"),
            Self::DuplicateNode { name } => {
                write!(f, "node名 '{name}' が重複しています")
            }
            Self::MissingFacet { node } => {
                write!(f, "node '{node}' にはファセット参照が必要です")
            }
            Self::EmptyFanoutChildren { node } => {
                write!(f, "fanout node '{node}' must reference at least one child")
            }
            Self::UnknownFanoutChild { node, child } => {
                write!(f, "fanout node '{node}' references unknown child node '{child}'")
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
            Self::FanoutChildLeafViolation {
                node,
                child,
                reason,
            } => {
                write!(
                    f,
                    "fanout node '{node}' child '{child}' violates leaf constraints: {reason}"
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
            Self::InvalidPermissionMode { node, value } => {
                let display_value = if value.is_empty() {
                    "(empty)"
                } else {
                    value.as_str()
                };
                write!(
                    f,
                    "node '{node}' のpermissionが不正です: invalid permission mode: {display_value} (allowed: {})",
                    ALLOWED_PERMISSION_MODES
                )
            }
            Self::MissingPermissionMode { node } => {
                write!(
                    f,
                    "node '{node}' にはpermissionが必要です (allowed: {})",
                    ALLOWED_PERMISSION_MODES
                )
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
            Self::UnknownModel { node, value } => {
                write!(
                    f,
                    "node '{node}' のmodelが不正です: unknown model: {value}"
                )
            }
            Self::InvalidModelFormat {
                node,
                value,
                reason,
            } => {
                write!(
                    f,
                    "node '{node}' のmodel '{value}' は形式として無効です: {reason}"
                )
            }
            Self::ModelResolutionFailed {
                node,
                value,
                reason,
            } => {
                write!(
                    f,
                    "node '{node}' のmodel '{value}' の所属バックエンドを解決できません: {reason}"
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
                    "command node '{node}' の artifact '{contract}' が予約 field '{field}' を宣言しています"
                )
            }
            Self::InvalidArtifactReference {
                reference, reason, ..
            } => {
                write!(f, "Artifact参照 '{reference}' が不正です: {reason}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

fn reference_error_to_validation_error(
    error: reference::ReferenceResolveError,
    context: reference::ReferenceResolveContext,
) -> ValidationError {
    match error {
        reference::ReferenceResolveError::ReservedNodeName { name } => {
            ValidationError::InvalidArtifactReference {
                reference: name,
                kind: InvalidArtifactReferenceKind::ReservedArtifactName,
                reason: "`request` and `item` are reserved Artifact names and cannot be node names"
                    .to_string(),
            }
        }
        reference::ReferenceResolveError::UnknownNode { name } => {
            ValidationError::InvalidArtifactReference {
                reference: name,
                kind: InvalidArtifactReferenceKind::UnknownNode,
                reason: "unknown Artifact-producing node".to_string(),
            }
        }
        reference::ReferenceResolveError::UnavailableArtifact { name } => {
            ValidationError::InvalidArtifactReference {
                reference: name,
                kind: InvalidArtifactReferenceKind::UnavailableArtifact,
                reason: "the referenced node does not produce an Artifact".to_string(),
            }
        }
        reference::ReferenceResolveError::UnknownField { reference, field } => {
            ValidationError::InvalidArtifactReference {
                reference: format!("{reference}.{field}"),
                kind: InvalidArtifactReferenceKind::UnknownField,
                reason: "unknown Artifact field".to_string(),
            }
        }
        reference::ReferenceResolveError::ItemOutOfScope => {
            ValidationError::InvalidArtifactReference {
                reference: reference::ITEM_ARTIFACT.to_string(),
                kind: InvalidArtifactReferenceKind::ItemOutOfScope,
                reason: "`item` is only available inside fanout child scope".to_string(),
            }
        }
        reference::ReferenceResolveError::InvalidInputRef { value } => {
            ValidationError::InvalidArtifactReference {
                reference: value,
                kind: InvalidArtifactReferenceKind::InvalidInputRef,
                reason: match context {
                    reference::ReferenceResolveContext::Inputs => {
                        "`inputs:` entries must be `request` or a top-level node Artifact name"
                    }
                    reference::ReferenceResolveContext::Template => {
                        "`{{ ... }}` references must be `request`, `item[.field]`, or `<node>[.field]`"
                    }
                }
                .to_string(),
            }
        }
        reference::ReferenceResolveError::InputsNotAllowedOnFanout { node } => {
            ValidationError::InvalidArtifactReference {
                reference: node,
                kind: InvalidArtifactReferenceKind::InputsNotAllowedOnFanout,
                reason: "fanout nodes cannot declare `inputs:`".to_string(),
            }
        }
    }
}

fn reference_diagnostic_to_validation_error(
    diagnostic: reference::ReferenceResolveDiagnostic,
) -> ValidationError {
    reference_error_to_validation_error(diagnostic.error, diagnostic.context)
}

fn routing_error_to_validation_error(error: routing::RoutingValidationError) -> ValidationError {
    match error {
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
        routing::RoutingValidationError::DiscriminatorOnFanout { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::DiscriminatorOnFanout,
                reason: "fanout nodes cannot use when or switch rules".to_string(),
            }
        }
        routing::RoutingValidationError::DiscriminatorWithoutArtifact { node } => {
            ValidationError::InvalidRules {
                node,
                kind: InvalidRuleKind::DiscriminatorWithoutArtifact,
                reason: "nodes without an artifact cannot use when or switch rules".to_string(),
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
        routing::RoutingValidationError::FanoutChildLeafViolation {
            fanout,
            child,
            reason,
        } => ValidationError::FanoutChildLeafViolation {
            node: fanout,
            child,
            reason,
        },
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

/// fanout child は top-level NodeDefinition の名前参照なので、定義数には重ねて数えない。
fn total_node_count(workflow: &WorkflowDefinition) -> usize {
    workflow.nodes.len()
}

fn collect_fanout_definition_errors(workflow: &WorkflowDefinition) -> Vec<ValidationError> {
    let node_by_name: HashMap<_, _> = workflow
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let mut errors = Vec::new();

    for parent in &workflow.nodes {
        let Some(fanout) = parent.fanout() else {
            continue;
        };
        if fanout.child.is_empty() {
            errors.push(ValidationError::EmptyFanoutChildren {
                node: parent.name.clone(),
            });
            continue;
        }

        let resolved_children = fanout
            .child
            .iter()
            .filter_map(
                |child_name| match node_by_name.get(child_name.as_str()).copied() {
                    Some(child) => Some(child),
                    None => {
                        errors.push(ValidationError::UnknownFanoutChild {
                            node: parent.name.clone(),
                            child: child_name.clone(),
                        });
                        None
                    }
                },
            )
            .collect::<Vec<_>>();

        match &fanout.items {
            None => {
                for child in resolved_children {
                    if child.input.is_some() {
                        errors.push(ValidationError::FanoutInputMismatch {
                            node: parent.name.clone(),
                            child: child.name.clone(),
                            reason: "child declares input but fanout has no items".to_string(),
                        });
                    }
                }
            }
            Some(items) => {
                for child in &resolved_children {
                    if child.input.is_none() {
                        errors.push(ValidationError::FanoutInputMismatch {
                            node: parent.name.clone(),
                            child: child.name.clone(),
                            reason: "fanout supplies items but child does not declare input"
                                .to_string(),
                        });
                    }
                }

                match items {
                    ItemsSource::ArtifactField { node, field } => {
                        let reference_value = format!("{node}.{field}");
                        match reference::artifact_field_schema(workflow, node, field) {
                            Err(error) => {
                                let converted = reference_error_to_validation_error(
                                    error,
                                    reference::ReferenceResolveContext::Template,
                                );
                                let reason = match converted {
                                    ValidationError::InvalidArtifactReference {
                                        reason, ..
                                    } => reason,
                                    _ => "unresolvable Artifact field".to_string(),
                                };
                                errors.push(ValidationError::InvalidFanoutItemsReference {
                                    node: parent.name.clone(),
                                    reference: reference_value,
                                    reason,
                                });
                            }
                            Ok(Some(SchemaDef::Array {
                                items: element_contract,
                            })) => {
                                for child in &resolved_children {
                                    if let Some(input_contract) = child.input.as_deref() {
                                        if input_contract != element_contract {
                                            errors.push(ValidationError::FanoutInputMismatch {
                                                node: parent.name.clone(),
                                                child: child.name.clone(),
                                                reason: format!(
                                                    "items element Contract '{element_contract}' does not match child input Contract '{input_contract}'"
                                                ),
                                            });
                                        }
                                    }
                                }
                            }
                            Ok(Some(_)) | Ok(None) => {
                                for child in &resolved_children {
                                    if child.input.is_some() {
                                        errors.push(ValidationError::FanoutInputMismatch {
                                            node: parent.name.clone(),
                                            child: child.name.clone(),
                                            reason: format!(
                                                "items reference '{reference_value}' must resolve to an array field"
                                            ),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    ItemsSource::Literal(values) => {
                        for child in &resolved_children {
                            let Some(input_contract) = child.input.as_deref() else {
                                continue;
                            };
                            let Some(schema) = workflow.schemas.get(input_contract) else {
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
                                    child: child.name.clone(),
                                    reason: format!(
                                        "literal item at index {item_index} does not match Contract '{input_contract}': {reason}"
                                    ),
                                });
                            }
                        }
                    }
                }
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
    if let Some(err) = collect_node_count_errors(workflow).into_iter().next() {
        return Err(err);
    }
    validate_schema_refs(workflow)?;
    if let Some(err) = reference::validate_workflow_reference_diagnostics(workflow)
        .into_iter()
        .next()
    {
        return Err(reference_diagnostic_to_validation_error(err));
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
    if let Some(error) = collect_fanout_definition_errors(workflow)
        .into_iter()
        .next()
    {
        return Err(error);
    }
    if let Some(err) = routing::validate_rules(workflow).into_iter().next() {
        return Err(routing_error_to_validation_error(err));
    }

    for node in &workflow.nodes {
        validate_node_kind_fields(node)?;
        if node.is_session() {
            validate_required_permission(&node.name, node.permission())?;
        }
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
            node.input.as_deref(),
            node.is_command(),
            node.is_fanout(),
            workflow,
        )?;
    }

    Ok(())
}

pub fn validate_template_references(
    workflow: &WorkflowDefinition,
    content: &str,
    allow_item: bool,
) -> Vec<ValidationError> {
    reference::validate_template_references(workflow, content, allow_item)
        .into_iter()
        .map(|error| {
            reference_error_to_validation_error(error, reference::ReferenceResolveContext::Template)
        })
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
    input: Option<&str>,
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

    if let Some(contract) = input {
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

/// node 数上限 (`MAX_NODES_PER_WORKFLOW`) と fanout child 参照数上限
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
            if fanout.child.len() > MAX_FANOUT_CHILDREN {
                errors.push(ValidationError::TooManyFanoutChildren {
                    node: node.name.clone(),
                    count: fanout.child.len(),
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
        NodeKindName::Session | NodeKindName::Fanout => {}
    }
    Ok(())
}

/// node に permission が必須として指定されていることを検証する。
/// `None` または対象外の値（旧語彙・未知語彙・空文字）はバリデーションエラー。
fn validate_required_permission(
    node_name: &str,
    value: Option<&str>,
) -> Result<(), ValidationError> {
    match value {
        None => Err(ValidationError::MissingPermissionMode {
            node: node_name.to_string(),
        }),
        Some(v) => {
            if !is_allowed_permission_mode(v) {
                return Err(ValidationError::InvalidPermissionMode {
                    node: node_name.to_string(),
                    value: v.to_string(),
                });
            }
            Ok(())
        }
    }
}

fn is_allowed_permission_mode(value: &str) -> bool {
    matches!(value, "ask" | "edit" | "full")
}

/// ワークフロー内の全 node の `model` フィールドを検証する。
///
/// 検証は経路によらず同一の基準で行う:
/// 1. 形式検証（`crate::domain::agent_session::ModelId`）— 空文字・空白のみ・制御文字・
///    上限長超過は登録判定に進まず形式不正として拒否する
/// 2. 登録判定（呼び出し側の resolver）— 未登録なら `UnknownModel`
/// 3. 所属解決（呼び出し側の resolver）— 複数 backend に登録された曖昧な model は拒否する
pub fn validate_models<F>(
    workflow: &WorkflowDefinition,
    mut resolve_model: F,
) -> Result<(), ValidationError>
where
    F: FnMut(&str) -> Result<Option<String>, String>,
{
    for node in &workflow.nodes {
        if let Some(model) = node.model() {
            validate_model_format(&node.name, model)?;
            validate_model_registered(&node.name, model, &mut resolve_model)?;
        }
    }
    Ok(())
}

fn validate_model_format(node_name: &str, model: &str) -> Result<(), ValidationError> {
    crate::domain::agent_session::ModelId::parse(model).map_err(|reason| {
        ValidationError::InvalidModelFormat {
            node: node_name.to_string(),
            value: model.to_string(),
            reason,
        }
    })?;
    Ok(())
}

fn validate_model_registered<F>(
    node_name: &str,
    model: &str,
    resolve_model: &mut F,
) -> Result<(), ValidationError>
where
    F: FnMut(&str) -> Result<Option<String>, String>,
{
    match resolve_model(model) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ValidationError::UnknownModel {
            node: node_name.to_string(),
            value: model.to_string(),
        }),
        Err(reason) => Err(ValidationError::ModelResolutionFailed {
            node: node_name.to_string(),
            value: model.to_string(),
            reason,
        }),
    }
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
            .map(reference_diagnostic_to_validation_error),
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
    errors.extend(collect_fanout_definition_errors(workflow));
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

    for node in &workflow.nodes {
        if let Err(e) = validate_node_kind_fields(node) {
            errors.push(e);
        }
        if node.is_session() {
            if let Err(e) = validate_required_permission(&node.name, node.permission()) {
                errors.push(e);
            }
        }
        if let Some(err) = check_missing_facet(node) {
            errors.push(err);
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        CommandSpec, FacetRefs, FanoutSpec, ItemsSource, NodeKind, Rule, SchemaDef, SessionGate,
        SessionSpec,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Copy)]
    enum TestKind {
        Session,
        ApprovalSession,
    }

    fn make_workflow_exact(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            schemas: Default::default(),
            name: "test".to_string(),
            description: "test workflow".to_string(),
            builtin: false,
            nodes,
        }
    }

    fn make_workflow(mut nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        // Most tests care about the fanout parent. Materialize referenced children as ordinary
        // top-level nodes unless a test supplied a customized definition explicitly.
        let existing: HashSet<String> = nodes.iter().map(|node| node.name.clone()).collect();
        let missing_children: Vec<String> = nodes
            .iter()
            .filter_map(NodeDefinition::fanout)
            .flat_map(|fanout| fanout.child.iter().cloned())
            .filter(|name| !existing.contains(name))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        nodes.extend(
            missing_children
                .iter()
                .map(|name| make_node(name, TestKind::Session, vec![])),
        );
        make_workflow_exact(nodes)
    }

    fn resolve_from_set(valid: &HashSet<String>, model: &str) -> Result<Option<String>, String> {
        Ok(valid.contains(model).then(|| "backend".to_string()))
    }

    fn make_node(name: &str, kind: TestKind, rules: Vec<Rule>) -> NodeDefinition {
        let node_kind = match kind {
            TestKind::Session => NodeKind::Session(SessionSpec {
                permission: Some("edit".to_string()),
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            TestKind::ApprovalSession => NodeKind::Session(SessionSpec {
                permission: Some("edit".to_string()),
                gate: SessionGate::Approval,
                facets: FacetRefs {
                    instruction: Some("implement".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
        };
        NodeDefinition {
            name: name.to_string(),
            kind: node_kind,
            rules,
            ..NodeDefinition::default()
        }
    }

    fn make_fanout_node(name: &str) -> String {
        name.to_string()
    }

    fn make_fanout_node_block(name: &str, children: Vec<String>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: children,
                items: None,
            }),
            ..NodeDefinition::default()
        }
    }

    fn with_fanout_items(mut node: NodeDefinition, items: ItemsSource) -> NodeDefinition {
        let NodeKind::Fanout(fanout) = &mut node.kind else {
            panic!("test node must be fanout");
        };
        fanout.items = Some(items);
        node
    }

    fn with_session_facets(mut node: NodeDefinition, facets: FacetRefs) -> NodeDefinition {
        node.session_mut()
            .expect("test node must be session")
            .facets = facets;
        node
    }

    fn without_session_facets(node: NodeDefinition) -> NodeDefinition {
        with_session_facets(node, FacetRefs::default())
    }

    fn with_session_permission(
        mut node: NodeDefinition,
        permission: Option<&str>,
    ) -> NodeDefinition {
        node.session_mut()
            .expect("test node must be session")
            .permission = permission.map(str::to_string);
        node
    }

    fn with_session_model(mut node: NodeDefinition, model: Option<&str>) -> NodeDefinition {
        node.session_mut().expect("test node must be session").model = model.map(str::to_string);
        node
    }

    fn with_input(mut node: NodeDefinition, input: &str) -> NodeDefinition {
        node.input = Some(input.to_string());
        node
    }

    fn command_node(name: &str, command: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: command.to_string(),
            }),
            ..Default::default()
        }
    }

    fn artifact_object_schema(fields: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties: fields
                .iter()
                .map(|field| ((*field).to_string(), SchemaDef::String { r#enum: None }))
                .collect(),
            required: BTreeSet::new(),
            additional_properties: false,
        }
    }

    fn workflow_with_schemas(
        nodes: Vec<NodeDefinition>,
        schemas: BTreeMap<String, SchemaDef>,
    ) -> WorkflowDefinition {
        WorkflowDefinition {
            schemas,
            ..make_workflow(nodes)
        }
    }

    // ---- 既存テスト ----

    #[test]
    fn validation_messages_use_node_vocabulary() {
        let errors = [
            ValidationError::EmptyNodes,
            ValidationError::DuplicateNode {
                name: "review".to_string(),
            },
            ValidationError::MissingFacet {
                node: "review".to_string(),
            },
            ValidationError::EmptyCommand {
                node: "build".to_string(),
            },
        ];

        for error in errors {
            let message = error.to_string();
            assert!(message.contains("node"), "unexpected message: {message}");
            assert!(!message.contains("ステップ"), "legacy message: {message}");
        }
    }

    #[test]
    fn valid_workflow_passes() {
        let wf = make_workflow(vec![
            make_node("plan", TestKind::ApprovalSession, vec![]),
            NodeDefinition {
                rules: vec![
                    Rule::LoopGuard {
                        max_iterations: 3,
                        on_exhausted: "plan".to_string(),
                    },
                    Rule::Next("plan".to_string()),
                ],
                ..make_node("implement", TestKind::Session, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    // [02]: Interactive 概念が廃止されたため、
    // 旧テスト `interactive_mode_fails_validation` は削除した。

    #[test]
    fn approval_gated_session_allows_terminal_rules_empty() {
        let wf = make_workflow(vec![
            make_node("fix", TestKind::Session, vec![]),
            make_node("approval", TestKind::ApprovalSession, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn rules_reject_multiple_next_catch_alls() {
        let wf = make_workflow(vec![
            make_node("fix", TestKind::Session, vec![]),
            make_node(
                "route",
                TestKind::ApprovalSession,
                vec![Rule::Next("fix".to_string()), Rule::Next("fix".to_string())],
            ),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidRules { ref node, .. } if node == "route"
        ));
    }

    #[test]
    fn rules_reject_standalone_next_with_discriminator() {
        let wf = make_workflow(vec![
            make_node("fix", TestKind::Session, vec![]),
            NodeDefinition {
                artifact: Some("verdict".to_string()),
                rules: vec![
                    Rule::When {
                        on: "ok".to_string(),
                        then: "fix".to_string(),
                        next: "fix".to_string(),
                    },
                    Rule::Next("fix".to_string()),
                ],
                ..make_node("route", TestKind::Session, vec![])
            },
        ]);
        let wf = workflow_with_schemas(
            wf.nodes,
            BTreeMap::from([(
                "verdict".to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([("ok".to_string(), SchemaDef::Boolean)]),
                    required: BTreeSet::from(["ok".to_string()]),
                    additional_properties: true,
                },
            )]),
        );
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidRules { ref node, .. } if node == "route"
        ));
    }

    #[test]
    fn invalid_transition_target_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            rules: vec![Rule::Next("nonexistent".to_string())],
            ..make_node("plan", TestKind::Session, vec![])
        }]);
        let result = validate(&wf);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownRuleTarget { ref target, .. } if target == "nonexistent"
        ));
    }

    #[test]
    fn routing_errors_map_by_variant_not_reason_text() {
        let err =
            routing_error_to_validation_error(routing::RoutingValidationError::UnknownRuleTarget {
                node: "route".to_string(),
                target: "missing".to_string(),
            });
        assert!(matches!(
            err,
            ValidationError::UnknownRuleTarget { ref node, ref target }
                if node == "route" && target == "missing"
        ));

        let err = routing_error_to_validation_error(
            routing::RoutingValidationError::MultipleNextCatchAll {
                node: "route".to_string(),
            },
        );
        assert!(matches!(
            err,
            ValidationError::InvalidRules { ref node, kind, .. }
                if node == "route" && kind == InvalidRuleKind::MultipleNextCatchAll
        ));
    }

    #[test]
    fn empty_nodes_fails() {
        let wf = make_workflow(vec![]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::EmptyNodes
        ));
    }

    #[test]
    fn valid_name_passes() {
        assert!(validate_name("quick-fix").is_ok());
        assert!(validate_name("my_workflow_v2").is_ok());
        assert!(validate_name("test123").is_ok());
    }

    #[test]
    fn invalid_name_with_traversal() {
        assert!(matches!(
            validate_name("../evil").unwrap_err(),
            ValidationError::InvalidChars { .. }
        ));
        assert!(validate_name("foo/bar").is_err());
        assert!(validate_name("..").is_err());
    }

    #[test]
    fn invalid_name_with_special_chars() {
        assert!(validate_name("foo bar").is_err());
        assert!(validate_name("foo.yml").is_err());
        assert!(matches!(
            validate_name("").unwrap_err(),
            ValidationError::EmptyName
        ));
    }

    #[test]
    fn invalid_name_leading_hyphen_or_underscore() {
        assert!(validate_name("-leading-hyphen").is_err());
        assert!(validate_name("_leading-underscore").is_err());
    }

    #[test]
    fn duplicate_node_names_fails() {
        let wf = make_workflow(vec![
            make_node("plan", TestKind::ApprovalSession, vec![]),
            make_node("plan", TestKind::Session, vec![]),
        ]);
        let result = validate(&wf);
        assert!(matches!(
            result.unwrap_err(),
            ValidationError::DuplicateNode { ref name } if name == "plan"
        ));
    }

    #[test]
    fn missing_facet_fails() {
        let wf = make_workflow(vec![without_session_facets(make_node(
            "node1",
            TestKind::Session,
            vec![],
        ))]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::MissingFacet { ref node } if node == "node1"
        ));
    }

    #[test]
    fn facet_only_node_passes() {
        let wf = make_workflow(vec![with_session_facets(
            make_node("node1", TestKind::Session, vec![]),
            FacetRefs {
                policy: Some("coding".to_string()),
                instruction: Some("implement".to_string()),
                ..Default::default()
            },
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn reserved_artifact_names_cannot_be_nodes() {
        for name in ["request", "item"] {
            let wf = make_workflow(vec![make_node(name, TestKind::Session, vec![])]);
            assert!(matches!(
                validate(&wf).unwrap_err(),
                ValidationError::InvalidArtifactReference { ref reference, .. } if reference == name
            ));
        }
    }

    #[test]
    fn unknown_input_artifact_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            inputs: vec!["nonexistent".to_string()],
            ..make_node("node1", TestKind::Session, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. } if reference == "nonexistent"
        ));
    }

    #[test]
    fn input_rejects_field_reference_item_and_request_field() {
        let mut schemas = BTreeMap::new();
        schemas.insert("plan-doc".to_string(), artifact_object_schema(&["summary"]));
        for input in ["plan.summary", "item", "request.field"] {
            let mut plan = make_node("plan", TestKind::Session, vec![]);
            plan.artifact = Some("plan-doc".to_string());
            let wf = workflow_with_schemas(
                vec![
                    plan,
                    NodeDefinition {
                        inputs: vec![input.to_string()],
                        ..make_node("consume", TestKind::Session, vec![])
                    },
                ],
                schemas.clone(),
            );

            assert!(matches!(
                validate(&wf).unwrap_err(),
                ValidationError::InvalidArtifactReference { ref reference, .. } if reference == input
            ));
        }
    }

    #[test]
    fn invalid_input_reference_keeps_inputs_context_reason() {
        let wf = make_workflow(vec![NodeDefinition {
            inputs: vec!["bad ref".to_string()],
            ..make_node("consume", TestKind::Session, vec![])
        }]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason, .. }
                if reference == "bad ref"
                    && reason == "`inputs:` entries must be `request` or a top-level node Artifact name"
        ));
    }

    #[test]
    fn invalid_template_reference_uses_template_context_reason() {
        let wf = make_workflow(vec![command_node("node1", "echo {{ bad ref }}")]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason, .. }
                if reference == "bad ref"
                    && reason.contains("{{ ... }}")
                    && !reason.contains("inputs:")
        ));
    }

    #[test]
    fn validate_template_references_uses_template_context_reason() {
        let wf = make_workflow(vec![make_node("review", TestKind::Session, vec![])]);
        let errors = validate_template_references(&wf, "{{ bad ref }}", false);

        assert!(matches!(
            errors.as_slice(),
            [ValidationError::InvalidArtifactReference { reference, reason, .. }]
                if reference == "bad ref"
                    && reason.contains("{{ ... }}")
                    && !reason.contains("inputs:")
        ));
    }

    #[test]
    fn legacy_task_template_reference_fails_when_no_artifact_exists() {
        let wf = make_workflow(vec![command_node("node1", "echo {{ task }}")]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. } if reference == "task"
        ));
    }

    #[test]
    fn item_template_reference_fails_outside_fanout_child_scope() {
        let wf = make_workflow(vec![command_node("node1", "echo {{ item.path }}")]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. } if reference == "item"
        ));
    }

    #[test]
    fn artifact_reference_to_session_without_artifact_fails() {
        let wf = make_workflow(vec![
            make_node("plan", TestKind::Session, vec![]),
            command_node("consume", "echo {{ plan }}"),
        ]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason, .. }
                if reference == "plan" && reason.contains("does not produce")
        ));
    }

    #[test]
    fn command_without_artifact_rejects_non_reserved_field() {
        let wf = make_workflow(vec![
            command_node("build", "cargo build"),
            command_node("consume", "echo {{ build.no_such_field }}"),
        ]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason, .. }
                if reference == "build.no_such_field" && reason.contains("unknown Artifact field")
        ));
    }

    #[test]
    fn artifact_node_rejects_undeclared_field() {
        let mut schemas = BTreeMap::new();
        schemas.insert("plan-doc".to_string(), artifact_object_schema(&["summary"]));
        let mut plan = make_node("plan", TestKind::Session, vec![]);
        plan.artifact = Some("plan-doc".to_string());
        let wf = workflow_with_schemas(
            vec![
                plan,
                command_node("consume", "echo {{ plan.unknown_field }}"),
            ],
            schemas,
        );

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason, .. }
                if reference == "plan.unknown_field" && reason.contains("unknown Artifact field")
        ));
    }

    #[test]
    fn valid_input_reference_passes() {
        let wf = make_workflow(vec![
            make_node("node_a", TestKind::Session, vec![]),
            NodeDefinition {
                ..make_node("node_b", TestKind::Session, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- 並列ブロック固有テスト ----

    #[test]
    fn valid_fanout_block_passes() {
        let wf = make_workflow(vec![
            make_node("implement", TestKind::Session, vec![]),
            make_fanout_node_block(
                "fanout-review",
                vec![
                    make_fanout_node("arch-review"),
                    make_fanout_node("security-review"),
                ],
            ),
            make_node("report", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn fanout_child_is_an_ordinary_top_level_node_reference() {
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("conflict")]),
            make_node("conflict", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn undefined_fanout_child_fails() {
        let wf = make_workflow_exact(vec![make_fanout_node_block(
            "par",
            vec!["missing".to_string()],
        )]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownFanoutChild { ref node, ref child }
                if node == "par" && child == "missing"
        ));
    }

    #[test]
    fn fanout_inputs_are_rejected() {
        let mut fanout = make_fanout_node_block(
            "par",
            vec![make_fanout_node("child1"), make_fanout_node("child2")],
        );
        fanout.inputs = vec!["request".to_string()];
        let wf = make_workflow(vec![fanout]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. } if reference == "par"
        ));
    }

    #[test]
    fn fanout_child_missing_facet_uses_normal_node_validation() {
        let child = without_session_facets(make_node("child1", TestKind::Session, vec![]));
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            child,
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::MissingFacet { ref node } if node == "child1"
        ));
    }

    // 新 schema では node_type が型レベルで必須となるため、旧テスト
    // `normal_node_missing_mode_fails` は YAML deserialize 段階で吸収される（[02] 範囲）。

    #[test]
    fn fanout_child_reference_valid_global_node() {
        let wf = make_workflow(vec![
            make_node("plan", TestKind::Session, vec![]),
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn fanout_child_artifact_reference_fails() {
        let mut child = make_node("arch-review", TestKind::Session, vec![]);
        child.artifact = Some("review-output".to_string());
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("arch-review")]),
            child,
            command_node("consume", "echo {{ arch-review.verdict }}"),
        ]);
        let wf = workflow_with_schemas(
            wf.nodes,
            BTreeMap::from([(
                "review-output".to_string(),
                artifact_object_schema(&["verdict"]),
            )]),
        );

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. }
                if reference == "arch-review"
        ));
    }

    #[test]
    fn empty_fanout_children_fails() {
        let wf = make_workflow(vec![
            make_node("implement", TestKind::Session, vec![]),
            make_fanout_node_block("fanout-review", vec![]),
            make_node("report", TestKind::Session, vec![]),
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::EmptyFanoutChildren { ref node }
                if node == "fanout-review"
        ));
    }

    #[test]
    fn fanout_without_items_rejects_child_input() {
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child")]),
            with_input(make_node("child", TestKind::Session, vec![]), "target"),
        ]);
        let wf = workflow_with_schemas(
            wf.nodes,
            BTreeMap::from([("target".to_string(), SchemaDef::String { r#enum: None })]),
        );

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::FanoutInputMismatch { ref node, ref child, .. }
                if node == "par" && child == "child"
        ));
    }

    #[test]
    fn fanout_with_items_requires_child_input() {
        let parent = with_fanout_items(
            make_fanout_node_block("par", vec![make_fanout_node("child")]),
            ItemsSource::Literal(vec![]),
        );
        let wf = make_workflow(vec![parent, make_node("child", TestKind::Session, vec![])]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::FanoutInputMismatch { ref node, ref child, .. }
                if node == "par" && child == "child"
        ));
    }

    #[test]
    fn fanout_literal_items_must_match_child_input_contract() {
        let parent = with_fanout_items(
            make_fanout_node_block("par", vec![make_fanout_node("child")]),
            ItemsSource::Literal(vec![serde_json::Value::String(
                "not-an-integer".to_string(),
            )]),
        );
        let wf = make_workflow(vec![
            parent,
            with_input(make_node("child", TestKind::Session, vec![]), "target"),
        ]);
        let wf = workflow_with_schemas(
            wf.nodes,
            BTreeMap::from([("target".to_string(), SchemaDef::Integer)]),
        );

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::FanoutInputMismatch { ref node, ref child, ref reason }
                if node == "par" && child == "child" && reason.contains("index 0")
        ));
    }

    #[test]
    fn fanout_artifact_items_element_contract_must_match_child_input() {
        let mut source = make_node("source", TestKind::Session, vec![]);
        source.artifact = Some("source-output".to_string());
        let parent = with_fanout_items(
            make_fanout_node_block("par", vec![make_fanout_node("child")]),
            ItemsSource::ArtifactField {
                node: "source".to_string(),
                field: "targets".to_string(),
            },
        );
        let wf = make_workflow(vec![
            source,
            parent,
            with_input(
                make_node("child", TestKind::Session, vec![]),
                "other-target",
            ),
        ]);
        let wf = workflow_with_schemas(
            wf.nodes,
            BTreeMap::from([
                (
                    "source-output".to_string(),
                    SchemaDef::Object {
                        properties: BTreeMap::from([(
                            "targets".to_string(),
                            SchemaDef::Array {
                                items: "target".to_string(),
                            },
                        )]),
                        required: BTreeSet::from(["targets".to_string()]),
                        additional_properties: false,
                    },
                ),
                ("target".to_string(), SchemaDef::String { r#enum: None }),
                ("other-target".to_string(), SchemaDef::Integer),
            ]),
        );

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::FanoutInputMismatch { ref node, ref child, ref reason }
                if node == "par" && child == "child" && reason.contains("target")
        ));
    }

    #[test]
    fn fanout_child_may_be_declared_after_parent() {
        let wf = make_workflow(vec![
            make_node("plan", TestKind::Session, vec![]),
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            make_node("report", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn fanout_child_rules_are_ignored_for_parent_routing() {
        let wf = make_workflow(vec![
            NodeDefinition {
                rules: vec![Rule::Next("done".to_string())],
                ..make_fanout_node_block("par", vec![make_fanout_node("child1")])
            },
            NodeDefinition {
                rules: vec![Rule::Next("par".to_string())],
                ..make_node("child1", TestKind::Session, vec![])
            },
            make_node("done", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn nested_fanout_child_fails() {
        let wf = make_workflow(vec![
            make_fanout_node_block("outer", vec![make_fanout_node("inner")]),
            make_fanout_node_block("inner", vec![make_fanout_node("leaf")]),
            make_node("leaf", TestKind::Session, vec![]),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::FanoutChildLeafViolation { ref node, ref child, .. }
                if node == "outer" && child == "inner"
        ));
    }

    #[test]
    fn fanout_child_cannot_be_workflow_entry() {
        let wf = make_workflow(vec![
            make_node("child", TestKind::Session, vec![]),
            make_fanout_node_block("par", vec![make_fanout_node("child")]),
        ]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::FanoutChildLeafViolation { ref node, ref child, ref reason }
                if node == "par" && child == "child" && reason.contains("entry")
        ));
    }

    #[test]
    fn fanout_child_cannot_be_normal_transition_target() {
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child")]),
            make_node("child", TestKind::Session, vec![]),
            make_node(
                "source",
                TestKind::Session,
                vec![Rule::Next("child".to_string())],
            ),
        ]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::FanoutChildLeafViolation { ref node, ref child, ref reason }
                if node == "par" && child == "child" && reason.contains("normal transition")
        ));
    }

    // ---- loop_guard / rules validation ----

    #[test]
    fn loop_guard_valid_target_passes() {
        let wf = make_workflow(vec![
            NodeDefinition {
                rules: vec![
                    Rule::LoopGuard {
                        max_iterations: 2,
                        on_exhausted: "approval".to_string(),
                    },
                    Rule::Next("approval".to_string()),
                ],
                ..make_node("fix", TestKind::Session, vec![])
            },
            make_node("approval", TestKind::ApprovalSession, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn loop_guard_unknown_target_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            rules: vec![Rule::LoopGuard {
                max_iterations: 2,
                on_exhausted: "nonexistent".to_string(),
            }],
            ..make_node("fix", TestKind::Session, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownRuleTarget { ref node, ref target }
                if node == "fix" && target == "nonexistent"
        ));
    }

    #[test]
    fn cycle_without_reachable_loop_guard_fails() {
        let wf = make_workflow(vec![
            make_node(
                "node_a",
                TestKind::Session,
                vec![Rule::Next("node_b".to_string())],
            ),
            make_node(
                "node_b",
                TestKind::Session,
                vec![Rule::Next("node_a".to_string())],
            ),
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidRules { reason, .. } if reason.contains("cycle reachable")
        ));
    }

    #[test]
    fn cycle_with_reachable_loop_guard_passes() {
        let wf = make_workflow(vec![
            make_node(
                "node_a",
                TestKind::Session,
                vec![Rule::Next("node_b".to_string())],
            ),
            make_node(
                "node_b",
                TestKind::Session,
                vec![
                    Rule::LoopGuard {
                        max_iterations: 2,
                        on_exhausted: "done".to_string(),
                    },
                    Rule::Next("node_a".to_string()),
                ],
            ),
            make_node("done", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- input_reference 後方参照 ----

    #[test]
    fn input_reference_backward_reference_passes() {
        // 定義順で後方の node を input_reference で参照できる
        let wf = make_workflow(vec![
            NodeDefinition {
                ..make_node("node_a", TestKind::Session, vec![])
            },
            make_node("node_b", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- permission バリデーション ----

    #[test]
    fn valid_permission_ask_passes() {
        let wf = make_workflow(vec![with_session_permission(
            make_node("node1", TestKind::Session, vec![]),
            Some("ask"),
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn valid_permission_edit_passes() {
        let wf = make_workflow(vec![with_session_permission(
            make_node("node1", TestKind::Session, vec![]),
            Some("edit"),
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn valid_permission_full_passes() {
        let wf = make_workflow(vec![with_session_permission(
            make_node("node1", TestKind::Session, vec![]),
            Some("full"),
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn legacy_permission_accept_edits_rejected() {
        for legacy in [
            "read",
            "readonly",
            "acceptEdits",
            "bypassPermissions",
            "plan",
            "default",
        ] {
            let wf = make_workflow(vec![with_session_permission(
                make_node("node1", TestKind::Session, vec![]),
                Some(legacy),
            )]);
            let err = validate(&wf).unwrap_err();
            assert!(matches!(
                err,
                ValidationError::InvalidPermissionMode { ref node, ref value }
                    if node == "node1" && value == legacy
            ));
            assert!(
                err.to_string().contains("ask, edit, full"),
                "error must include allowed list, got: {err}"
            );
        }
    }

    #[test]
    fn invalid_permission_fails() {
        let wf = make_workflow(vec![with_session_permission(
            make_node("node1", TestKind::Session, vec![]),
            Some("invalid-mode"),
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref node, ref value }
                if node == "node1" && value == "invalid-mode"
        ));
        assert!(err.to_string().contains("ask, edit, full"));
    }

    #[test]
    fn empty_permission_fails() {
        let wf = make_workflow(vec![with_session_permission(
            make_node("node1", TestKind::Session, vec![]),
            Some(""),
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref node, ref value }
                if node == "node1" && value.is_empty()
        ));
        assert!(err.to_string().contains("ask, edit, full"));
    }

    #[test]
    fn invalid_permission_on_fanout_child_fails() {
        let child = with_session_permission(
            make_node("child1", TestKind::Session, vec![]),
            Some("acceptEdits"),
        );
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            child,
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref node, ref value }
                if node == "child1" && value == "acceptEdits"
        ));
    }

    #[test]
    fn valid_permission_on_fanout_child_passes() {
        let child =
            with_session_permission(make_node("child1", TestKind::Session, vec![]), Some("full"));
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            child,
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn node_without_permission_fails() {
        let wf = make_workflow(vec![with_session_permission(
            make_node("node1", TestKind::Session, vec![]),
            None,
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MissingPermissionMode { ref node } if node == "node1"
        ));
        assert!(err.to_string().contains("ask, edit, full"));
    }

    #[test]
    fn fanout_child_without_permission_fails() {
        let child = with_session_permission(make_node("child1", TestKind::Session, vec![]), None);
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            child,
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MissingPermissionMode { ref node } if node == "child1"
        ));
    }

    #[test]
    fn fanout_block_without_permission_passes_when_children_have_permission() {
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            with_session_permission(make_node("child1", TestKind::Session, vec![]), Some("edit")),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- model バリデーション (validate_models) ----

    #[test]
    fn validate_models_valid_model_passes() {
        let wf = make_workflow(vec![with_session_model(
            make_node("node1", TestKind::Session, vec![]),
            Some("haiku"),
        )]);
        let valid = HashSet::from(["haiku".to_string(), "opus-4".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_unknown_model_fails() {
        let wf = make_workflow(vec![with_session_model(
            make_node("node1", TestKind::Session, vec![]),
            Some("unknown-model"),
        )]);
        let valid = HashSet::from(["haiku".to_string(), "opus-4".to_string()]);
        let err = validate_models(&wf, |model| resolve_from_set(&valid, model)).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownModel { ref node, ref value }
                if node == "node1" && value == "unknown-model"
        ));
        assert!(err.to_string().contains("unknown model: unknown-model"));
    }

    #[test]
    fn validate_models_unknown_model_on_fanout_child_fails() {
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            with_session_model(
                make_node("child1", TestKind::Session, vec![]),
                Some("unknown-model"),
            ),
        ]);
        let valid = HashSet::from(["haiku".to_string()]);
        let err = validate_models(&wf, |model| resolve_from_set(&valid, model)).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownModel { ref node, ref value }
                if node == "child1" && value == "unknown-model"
        ));
    }

    #[test]
    fn validate_models_rejects_ambiguous_model_from_resolver() {
        let wf = make_workflow(vec![with_session_model(
            make_node("node1", TestKind::Session, vec![]),
            Some("shared"),
        )]);
        let err = validate_models(&wf, |model| {
            Err(format!(
                "モデル '{model}' が複数のバックエンドに登録されています"
            ))
        })
        .unwrap_err();
        assert!(matches!(
            err,
            ValidationError::ModelResolutionFailed { ref node, ref value, ref reason }
                if node == "node1" && value == "shared" && reason.contains("複数")
        ));
    }

    #[test]
    fn validate_models_no_model_specified_passes() {
        let wf = make_workflow(vec![make_node("node1", TestKind::Session, vec![])]);
        let valid = HashSet::from(["haiku".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_valid_model_on_fanout_child_passes() {
        let wf = make_workflow(vec![
            make_fanout_node_block("par", vec![make_fanout_node("child1")]),
            with_session_model(
                make_node("child1", TestKind::Session, vec![]),
                Some("haiku"),
            ),
        ]);
        let valid = HashSet::from(["haiku".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_rejects_empty_model_before_registry_check() {
        // 形式不正（空文字）は registry に含まれるかにかかわらず拒否される。
        // 未登録（UnknownModel）と区別するため InvalidModelFormat として報告される。
        let wf = make_workflow(vec![with_session_model(
            make_node("node1", TestKind::Session, vec![]),
            Some(""),
        )]);
        // valid_models に空文字を含めても形式検証で先に弾く
        let valid = HashSet::from([String::new(), "haiku".to_string()]);
        let err = validate_models(&wf, |model| resolve_from_set(&valid, model)).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidModelFormat { ref node, ref value, .. }
                if node == "node1" && value.is_empty()
        ));
    }

    #[test]
    fn validate_models_rejects_whitespace_only_model() {
        let wf = make_workflow(vec![with_session_model(
            make_node("node1", TestKind::Session, vec![]),
            Some("   "),
        )]);
        let valid = HashSet::from(["   ".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_err());
    }

    #[test]
    fn validate_models_rejects_control_character_model() {
        let wf = make_workflow(vec![with_session_model(
            make_node("node1", TestKind::Session, vec![]),
            Some("a\u{0001}b"),
        )]);
        let valid = HashSet::from(["a\u{0001}b".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_err());
    }

    // ---- command kind validation ----

    #[test]
    fn command_node_with_command_passes_when_facets_absent() {
        let wf = make_workflow(vec![command_node("build", "cargo build")]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn command_node_with_empty_command_fails() {
        let wf = make_workflow(vec![command_node("build", "   ")]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::EmptyCommand { ref node } if node == "build"
        ));
    }

    fn object_schema(required: &[&str]) -> SchemaDef {
        SchemaDef::Object {
            properties: required
                .iter()
                .map(|field| ((*field).to_string(), SchemaDef::String { r#enum: None }))
                .collect(),
            required: required.iter().map(|field| (*field).to_string()).collect(),
            additional_properties: true,
        }
    }

    #[test]
    fn schema_refs_allow_session_artifact_and_input() {
        let mut session = make_node("review", TestKind::Session, vec![]);
        session.artifact = Some("review-output".to_string());
        session.input = Some("review-input".to_string());
        let mut wf = make_workflow(vec![session]);
        wf.schemas
            .insert("review-output".to_string(), object_schema(&["status"]));
        wf.schemas.insert(
            "review-input".to_string(),
            SchemaDef::String { r#enum: None },
        );

        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn schema_refs_reject_request_schema_but_allow_item_schema() {
        let mut request_wf = make_workflow(vec![make_node("review", TestKind::Session, vec![])]);
        request_wf
            .schemas
            .insert("request".to_string(), SchemaDef::String { r#enum: None });
        assert!(matches!(
            validate_schema_refs(&request_wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason, .. }
                if reference == "request" && reason.contains("reserved Artifact name")
        ));

        let mut item_wf = make_workflow(vec![make_node("review", TestKind::Session, vec![])]);
        item_wf
            .schemas
            .insert("item".to_string(), SchemaDef::String { r#enum: None });
        assert!(validate_schema_refs(&item_wf).is_ok());
    }

    #[test]
    fn schema_refs_reject_unknown_artifact_schema() {
        let mut session = make_node("review", TestKind::Session, vec![]);
        session.artifact = Some("missing".to_string());
        let wf = make_workflow(vec![session]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownSchemaRef { ref node, slot, ref key }
                if node == "review" && slot == "artifact" && key == "missing"
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_schema_identifier() {
        let mut session = make_node("review", TestKind::Session, vec![]);
        session.artifact = Some("review; curl https://example.invalid #".to_string());
        let mut wf = make_workflow(vec![session]);
        wf.schemas.insert(
            "review; curl https://example.invalid #".to_string(),
            object_schema(&["status"]),
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason, .. }
                if schema == "review; curl https://example.invalid #"
                    && reason.contains("must start with an ASCII alphanumeric")
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_artifact_reference_identifier() {
        let mut session = make_node("review", TestKind::Session, vec![]);
        session.artifact = Some("review; curl https://example.invalid #".to_string());
        let wf = make_workflow(vec![session]);

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchemaRef { ref node, slot, ref key, ref reason }
                if node == "review"
                    && slot == "artifact"
                    && key == "review; curl https://example.invalid #"
                    && reason.contains("must start with an ASCII alphanumeric")
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_input_reference_identifier() {
        let mut session = make_node("review", TestKind::Session, vec![]);
        session.input = Some("../outside".to_string());
        let wf = make_workflow(vec![session]);

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchemaRef { ref node, slot, ref key, .. }
                if node == "review" && slot == "input" && key == "../outside"
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_array_items_identifier() {
        let mut wf = make_workflow(vec![make_node("review", TestKind::Session, vec![])]);
        wf.schemas.insert(
            "review-list".to_string(),
            SchemaDef::Array {
                items: "../outside".to_string(),
            },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason, .. }
                if schema == "review-list"
                    && reason.contains("must start with an ASCII alphanumeric")
        ));
    }

    #[test]
    fn schema_refs_reject_required_field_missing_from_properties() {
        let mut wf = make_workflow(vec![make_node("review", TestKind::Session, vec![])]);
        wf.schemas.insert(
            "review-output".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::new(),
                required: BTreeSet::from(["verdict".to_string()]),
                additional_properties: true,
            },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason, .. }
                if schema == "review-output"
                    && reason == "required field 'verdict' is not declared in properties"
        ));
    }

    #[test]
    fn schema_refs_reject_empty_string_enum() {
        let mut wf = make_workflow(vec![make_node("review", TestKind::Session, vec![])]);
        wf.schemas.insert(
            "review-output".to_string(),
            SchemaDef::String {
                r#enum: Some(Vec::new()),
            },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason, .. }
                if schema == "review-output" && reason == "enum must contain at least one value"
        ));
    }

    #[test]
    fn schema_refs_reject_array_items_unknown_schema() {
        let mut wf = make_workflow(vec![make_node("review", TestKind::Session, vec![])]);
        wf.schemas.insert(
            "review-list".to_string(),
            SchemaDef::Array {
                items: "missing-item".to_string(),
            },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason, .. }
                if schema == "review-list"
                    && reason == "array.items references unknown schemas 'missing-item'"
        ));
    }

    #[test]
    fn schema_refs_reject_session_artifact_non_object_schema() {
        let mut session = make_node("review", TestKind::Session, vec![]);
        session.artifact = Some("review-output".to_string());
        let mut wf = make_workflow(vec![session]);
        wf.schemas.insert(
            "review-output".to_string(),
            SchemaDef::String { r#enum: None },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidArtifactSchema { ref node, ref contract }
                if node == "review" && contract == "review-output"
        ));
    }

    #[test]
    fn fanout_node_rejects_artifact_declaration() {
        let mut fanout = make_fanout_node_block("review", vec![make_fanout_node("review-a")]);
        fanout.artifact = Some("review-output".to_string());
        let mut wf = make_workflow(vec![fanout]);
        wf.schemas
            .insert("review-output".to_string(), object_schema(&["status"]));

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactSchema { ref node, ref contract }
                if node == "review" && contract == "review-output"
        ));
    }

    #[test]
    fn command_node_rejects_artifact_reserved_field_collision() {
        let mut command = command_node("build", "cargo build");
        command.artifact = Some("build-output".to_string());
        let mut wf = make_workflow(vec![command]);
        wf.schemas.insert(
            "build-output".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([("ok".to_string(), SchemaDef::Boolean)]),
                required: BTreeSet::from(["ok".to_string()]),
                additional_properties: true,
            },
        );

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ReservedArtifactField { ref node, ref contract, ref field }
                if node == "build" && contract == "build-output" && field == "ok"
        ));
    }

    // ---- DoS ガードのテスト ----

    #[test]
    fn too_many_nodes_fails() {
        let nodes: Vec<NodeDefinition> = (0..MAX_NODES_PER_WORKFLOW + 1)
            .map(|i| make_node(&format!("node{i}"), TestKind::Session, vec![]))
            .collect();
        let wf = make_workflow(nodes);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::TooManyNodes { .. }
        ));
    }

    #[test]
    fn fanout_references_are_not_counted_as_embedded_nodes() {
        let wf = make_workflow(vec![make_fanout_node_block(
            "par",
            vec![make_fanout_node("child")],
        )]);

        assert_eq!(total_node_count(&wf), 2);
    }

    #[test]
    fn too_many_fanout_children_fails() {
        let children: Vec<String> = (0..MAX_FANOUT_CHILDREN + 1)
            .map(|i| make_fanout_node(&format!("c{i}")))
            .collect();
        let wf = make_workflow(vec![make_fanout_node_block("par", children)]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::TooManyFanoutChildren { ref node, .. } if node == "par"
        ));
    }

    #[test]
    fn validate_schema_refs_inspects_top_level_fanout_children() {
        let child = NodeDefinition {
            input: Some("nope".to_string()),
            ..make_node("child1", TestKind::Session, vec![])
        };
        let par = make_fanout_node_block("par", vec![make_fanout_node("child1")]);
        let wf = make_workflow(vec![par, child]);
        let err = validate_schema_refs(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownSchemaRef { ref node, slot, ref key }
                if node == "child1" && slot == "input" && key == "nope"
        ));
    }
}
