use crate::domain::workflow::services::{contract_schema, reference, routing};
use crate::domain::workflow::value_objects::{MAX_NODES_PER_WORKFLOW, MAX_PARALLEL_CHILDREN};
use crate::domain::workflow::{
    NodeDefinition, NodeKindName, ReduceStrategy, SchemaDef, WorkflowDefinition as Workflow,
    WorkflowError, WorkflowName,
};
use regex::RegexBuilder;
use std::collections::HashSet;
use std::fmt;

const ALLOWED_PERMISSION_MODES: &str = "ask, edit, full";

#[derive(Debug)]
pub enum ValidationError {
    EmptyName,
    InvalidChars {
        name: String,
    },
    EmptySteps,
    DuplicateStep {
        name: String,
    },
    MissingFacet {
        step: String,
    },
    UnknownCollectFrom {
        step: String,
        reference: String,
    },
    /// 並列子step名がグローバル名前空間で重複
    ParallelChildNameConflict {
        child: String,
    },
    /// aggregate 設定が不正（all_match/any_match 排他違反等）
    AggregateInvalidConfig {
        step: String,
        reason: String,
    },
    /// aggregate の遷移先が存在しない
    AggregateUnknownTarget {
        step: String,
        target: String,
    },
    /// 並列子stepにファセット参照がない
    ParallelChildMissingFacet {
        parent: String,
        child: String,
    },
    /// rules の遷移先が存在しない node を参照
    UnknownRuleTarget {
        step: String,
        target: String,
    },
    /// rules の順序非依存性・網羅性・型付き参照・loop guard が不正
    InvalidRules {
        step: String,
        reason: String,
    },
    /// 無効な permission mode が指定されている
    InvalidPermissionMode {
        step: String,
        value: String,
    },
    /// step に permission が指定されていない（必須）
    MissingPermissionMode {
        step: String,
    },
    /// command 種別 node の `command` が空文字
    EmptyCommand {
        step: String,
    },
    /// node 種別ごとに許可されないフィールドが指定されている
    DisallowedFieldForKind {
        step: String,
        field: &'static str,
        kind: &'static str,
    },
    /// `nodes` の総数が DoS 防御の上限を超えた
    TooManyNodes {
        count: usize,
        max: usize,
    },
    /// `parallel_children` の数が DoS 防御の上限を超えた
    TooManyParallelChildren {
        step: String,
        count: usize,
        max: usize,
    },
    /// 存在しないモデルが指定されている
    UnknownModel {
        step: String,
        value: String,
    },
    /// モデルIDが形式として無効（空文字・空白のみ・制御文字・上限長超過など）。
    /// `reason` には `ModelId` の戻り値（理由文言）を保持し、
    /// 呼び出し側・ログで未登録（UnknownModel）と区別できるようにする。
    InvalidModelFormat {
        step: String,
        value: String,
        reason: String,
    },
    /// モデルIDの形式は有効だが、バックエンド所属を一意に解決できない。
    ModelResolutionFailed {
        step: String,
        value: String,
        reason: String,
    },
    /// `artifact` / `input` が存在しない `schemas:` Contract を参照している。
    UnknownSchemaRef {
        step: String,
        slot: &'static str,
        key: String,
    },
    /// `artifact` / `input` の Contract 参照名が安全な identifier ではない。
    InvalidSchemaRef {
        step: String,
        slot: &'static str,
        key: String,
        reason: String,
    },
    /// `schemas:` 内の宣言が JSON Schema subset として矛盾している。
    InvalidSchema {
        schema: String,
        reason: String,
    },
    /// `artifact:` が Object 以外の Contract を参照している。
    InvalidArtifactSchema {
        step: String,
        contract: String,
    },
    /// command node の `artifact:` Contract が予約 field を宣言している。
    ReservedArtifactField {
        step: String,
        contract: String,
        field: String,
    },
    /// `inputs:` または `{{ ... }}` が解決できない Artifact 参照を含む。
    InvalidArtifactReference {
        reference: String,
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
            Self::EmptySteps => write!(f, "ワークフローにステップが定義されていません"),
            Self::DuplicateStep { name } => {
                write!(f, "ステップ名 '{name}' が重複しています")
            }
            Self::MissingFacet { step } => {
                write!(
                    f,
                    "ステップ '{step}' にはファセット参照が必要です（collectステップのみ省略可）"
                )
            }
            Self::UnknownCollectFrom { step, reference } => write!(
                f,
                "ステップ '{step}' のcollect.fromが存在しないステップ '{reference}' を参照しています"
            ),
            Self::ParallelChildNameConflict { child } => {
                write!(f, "並列子ステップ名 '{child}' が他のステップ名と重複しています")
            }
            Self::AggregateInvalidConfig { step, reason } => {
                write!(f, "ステップ '{step}' のaggregate設定が不正です: {reason}")
            }
            Self::AggregateUnknownTarget { step, target } => write!(
                f,
                "ステップ '{step}' のaggregateが存在しないステップ '{target}' を参照しています"
            ),
            Self::ParallelChildMissingFacet { parent, child } => write!(
                f,
                "parallelブロック '{parent}' の子ステップ '{child}' にはファセット参照が必要です"
            ),
            Self::UnknownRuleTarget { step, target } => write!(
                f,
                "node '{step}' のrulesが存在しないnode '{target}' を参照しています"
            ),
            Self::InvalidRules { step, reason } => {
                write!(f, "node '{step}' のrulesが不正です: {reason}")
            }
            Self::InvalidPermissionMode { step, value } => {
                let display_value = if value.is_empty() {
                    "(empty)"
                } else {
                    value.as_str()
                };
                write!(
                    f,
                    "ステップ '{step}' のpermissionが不正です: invalid permission mode: {display_value} (allowed: {})",
                    ALLOWED_PERMISSION_MODES
                )
            }
            Self::MissingPermissionMode { step } => {
                write!(
                    f,
                    "ステップ '{step}' にはpermissionが必要です (allowed: {})",
                    ALLOWED_PERMISSION_MODES
                )
            }
            Self::TooManyNodes { count, max } => write!(
                f,
                "node 数 {count} がワークフローあたりの上限 {max} を超えています"
            ),
            Self::TooManyParallelChildren { step, count, max } => write!(
                f,
                "ステップ '{step}' の parallel_children の数 {count} が上限 {max} を超えています"
            ),
            Self::EmptyCommand { step } => {
                write!(
                    f,
                    "commandステップ '{step}' の command は空にできません"
                )
            }
            Self::DisallowedFieldForKind {
                step,
                field,
                kind,
            } => write!(
                f,
                "ステップ '{step}' ({kind}) には '{field}' を指定できません"
            ),
            Self::UnknownModel { step, value } => {
                write!(
                    f,
                    "ステップ '{step}' のmodelが不正です: unknown model: {value}"
                )
            }
            Self::InvalidModelFormat {
                step,
                value,
                reason,
            } => {
                write!(
                    f,
                    "ステップ '{step}' のmodel '{value}' は形式として無効です: {reason}"
                )
            }
            Self::ModelResolutionFailed {
                step,
                value,
                reason,
            } => {
                write!(
                    f,
                    "ステップ '{step}' のmodel '{value}' の所属バックエンドを解決できません: {reason}"
                )
            }
            Self::UnknownSchemaRef { step, slot, key } => {
                write!(
                    f,
                    "ステップ '{step}' の {slot} が存在しない schemas Contract '{key}' を参照しています"
                )
            }
            Self::InvalidSchemaRef {
                step,
                slot,
                key,
                reason,
            } => {
                write!(
                    f,
                    "ステップ '{step}' の {slot} Contract 参照 '{key}' が不正です: {reason}"
                )
            }
            Self::InvalidSchema { schema, reason } => {
                write!(f, "schemas.{schema} の宣言が不正です: {reason}")
            }
            Self::InvalidArtifactSchema { step, contract } => {
                write!(
                    f,
                    "ステップ '{step}' の artifact '{contract}' は Object Contract である必要があります"
                )
            }
            Self::ReservedArtifactField {
                step,
                contract,
                field,
            } => {
                write!(
                    f,
                    "commandステップ '{step}' の artifact '{contract}' が予約 field '{field}' を宣言しています"
                )
            }
            Self::InvalidArtifactReference { reference, reason } => {
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
                reason: "`request` and `item` are reserved Artifact names and cannot be node names"
                    .to_string(),
            }
        }
        reference::ReferenceResolveError::UnknownNode { name } => {
            ValidationError::InvalidArtifactReference {
                reference: name,
                reason: "unknown Artifact-producing node".to_string(),
            }
        }
        reference::ReferenceResolveError::UnavailableArtifact { name } => {
            ValidationError::InvalidArtifactReference {
                reference: name,
                reason: "the referenced node does not produce an Artifact".to_string(),
            }
        }
        reference::ReferenceResolveError::UnknownField { reference, field } => {
            ValidationError::InvalidArtifactReference {
                reference: format!("{reference}.{field}"),
                reason: "unknown Artifact field".to_string(),
            }
        }
        reference::ReferenceResolveError::ItemOutOfScope => {
            ValidationError::InvalidArtifactReference {
                reference: reference::ITEM_ARTIFACT.to_string(),
                reason: "`item` is only available inside fanout child scope".to_string(),
            }
        }
        reference::ReferenceResolveError::InvalidInputRef { value } => {
            ValidationError::InvalidArtifactReference {
                reference: value,
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
        routing::RoutingValidationError::UnknownRuleTarget { step, target } => {
            ValidationError::UnknownRuleTarget { step, target }
        }
        routing::RoutingValidationError::Invalid { step, reason } => {
            ValidationError::InvalidRules { step, reason }
        }
    }
}

pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    match WorkflowName::new(name) {
        Ok(_) => Ok(()),
        Err(_) if name.is_empty() => Err(ValidationError::EmptyName),
        Err(_) => Err(ValidationError::InvalidChars {
            name: name.to_string(),
        }),
    }
}

pub fn validate_workflow_shape(workflow: &Workflow) -> Result<(), WorkflowError> {
    if workflow.nodes.is_empty() {
        return Err(WorkflowError::validation("workflow has no nodes"));
    }
    if let Some(node) = workflow.nodes.iter().find(|node| node.is_command()) {
        return Err(WorkflowError::validation(format!(
            "command node '{}' is not executable in this milestone",
            node.name
        )));
    }
    Ok(())
}

/// `workflow.nodes` の top-level node 数と全 `parallel_children` の合算（=DoS ガード対象の総 node 数）。
fn total_node_count(workflow: &Workflow) -> usize {
    workflow.nodes.iter().fold(0usize, |acc, n| {
        let child_count = n
            .fanout()
            .map(|fanout| fanout.parallel_children.len())
            .unwrap_or(0);
        acc + 1 + child_count
    })
}

pub fn validate(workflow: &Workflow) -> Result<(), ValidationError> {
    validate_name(&workflow.name)?;

    if workflow.nodes.is_empty() {
        return Err(ValidationError::EmptySteps);
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

    // 遷移先名前空間: トップレベルstep名のみ（aggregate.then/else, rule.nextの検証用）
    let mut transition_target_names = HashSet::new();
    let mut referenceable_step_names = HashSet::new();
    for step in &workflow.nodes {
        if !transition_target_names.insert(step.name.as_str()) {
            return Err(ValidationError::DuplicateStep {
                name: step.name.clone(),
            });
        }
        referenceable_step_names.insert(step.name.as_str());
        // 並列子step名は参照可能名前空間にのみ追加（遷移先には不可）
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                if !referenceable_step_names.insert(child.name.as_str()) {
                    return Err(ValidationError::ParallelChildNameConflict {
                        child: child.name.clone(),
                    });
                }
            }
        }
    }
    if let Some(err) = routing::validate_rules(workflow).into_iter().next() {
        return Err(routing_error_to_validation_error(err));
    }

    // 各ステップより前に定義されたステップ名を追跡（collect.from検証用）
    let mut preceding_step_names: HashSet<&str> = HashSet::new();

    for step in &workflow.nodes {
        validate_node_kind_fields(step)?;
        if step.is_fanout() {
            // --- parallel block 固有のバリデーション ---
            // [02] では node_type=Parallel が型レベルで mode を排除するため、
            // 旧 schema の「parallel に mode 指定」エラーは存在しない。

            let children = &step
                .fanout()
                .expect("is_fanout checked above")
                .parallel_children;
            if children.is_empty() {
                return Err(ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "fanoutブロックには1つ以上の子ステップが必要です".to_string(),
                });
            }
            for child in children {
                // 子step にはファセット参照が必要
                if !child.has_facet_refs() {
                    return Err(ValidationError::ParallelChildMissingFacet {
                        parent: step.name.clone(),
                        child: child.name.clone(),
                    });
                }

                // 子step の permission 妥当性チェック（必須）
                validate_required_permission(&child.name, child.permission.as_deref())?;
            }

            // aggregate バリデーション
            if let Some(agg) = step.fanout().and_then(|fanout| fanout.aggregate.as_ref()) {
                // all_match と any_match の排他チェック
                match (&agg.all_match, &agg.any_match) {
                    (Some(_), Some(_)) => {
                        return Err(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: "all_matchとany_matchは同時に指定できません".to_string(),
                        });
                    }
                    (None, None) => {
                        return Err(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: "all_matchまたはany_matchのいずれかが必要です".to_string(),
                        });
                    }
                    _ => {}
                }

                // regex妥当性チェック
                if let Some(ref pattern) = agg.all_match {
                    if RegexBuilder::new(pattern)
                        .size_limit(1 << 20)
                        .build()
                        .is_err()
                    {
                        return Err(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: format!("all_matchのパターンが不正な正規表現です: {pattern}"),
                        });
                    }
                }
                if let Some(ref pattern) = agg.any_match {
                    if RegexBuilder::new(pattern)
                        .size_limit(1 << 20)
                        .build()
                        .is_err()
                    {
                        return Err(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: format!("any_matchのパターンが不正な正規表現です: {pattern}"),
                        });
                    }
                }

                // then/else の遷移先存在チェック（トップレベルstepのみ）
                if !transition_target_names.contains(agg.then.as_str()) {
                    return Err(ValidationError::AggregateUnknownTarget {
                        step: step.name.clone(),
                        target: agg.then.clone(),
                    });
                }
                if !transition_target_names.contains(agg.r#else.as_str()) {
                    return Err(ValidationError::AggregateUnknownTarget {
                        step: step.name.clone(),
                        target: agg.r#else.clone(),
                    });
                }
            }
        } else {
            // --- 通常step のバリデーション ---
            // 新 schema では kind が型レベルで必須・列挙のため、
            // 旧 schema の MissingMode / InteractiveModeNotAllowed 検査は
            // YAML deserialize 段階で吸収される（[02] 範囲外）。
            // permission の妥当性チェック（必須）
            if step.is_session() {
                validate_required_permission(&step.name, step.permission())?;
            }

            if let Some(err) = check_missing_facet(step) {
                return Err(err);
            }

            // collect.from の参照先 step 名が先行stepに存在するか検証（並列子stepも参照可）
            if let Some(ref collect) = step.collect {
                for r in &collect.from {
                    if !preceding_step_names.contains(r.as_str()) {
                        return Err(ValidationError::UnknownCollectFrom {
                            step: step.name.clone(),
                            reference: r.clone(),
                        });
                    }
                }

                // any_needs_fix / all_passed 使用時に参照先 step の rules 未定義を警告
                if matches!(
                    collect.reduce,
                    ReduceStrategy::AnyNeedsFix | ReduceStrategy::AllPassed
                ) {
                    for r in &collect.from {
                        let referenced_step = workflow.nodes.iter().find(|s| s.name == *r);
                        if let Some(rs) = referenced_step {
                            if rs.rules.is_empty() && !rs.is_fanout() {
                                log::warn!(
                                    "collect step '{}' uses {:?} reducer but source step '{}' has no rules defined (result may be None)",
                                    step.name,
                                    collect.reduce,
                                    r
                                );
                            }
                        }
                    }
                }
            }
        }

        // 次のステップのvalidationで使えるよう、このステップ名を追加
        preceding_step_names.insert(&step.name);
        // parallel blockの子step名も追加（後続parallel blockの子stepから参照可能にするため）
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }

    Ok(())
}

/// `schemas:` 宣言と `artifact` / `input` 参照を検証する。
///
/// validation.rs は facet I/O を持たず、workflow definition に含まれる
/// backend-owned schema state だけを読む。
pub fn validate_schema_refs(workflow: &Workflow) -> Result<(), ValidationError> {
    for (name, schema) in &workflow.schemas {
        if name == reference::REQUEST_ARTIFACT {
            return Err(ValidationError::InvalidArtifactReference {
                reference: name.to_string(),
                reason: "`request` is a reserved Artifact name and cannot be declared in schemas"
                    .to_string(),
            });
        }
        if !contract_schema::is_safe_identifier(name) {
            return Err(ValidationError::InvalidSchema {
                schema: name.to_string(),
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
        if let Some(fanout) = node.fanout() {
            for child in &fanout.parallel_children {
                validate_node_schema_refs(
                    &child.name,
                    child.artifact.as_deref(),
                    child.input.as_deref(),
                    false,
                    false,
                    workflow,
                )?;
            }
        }
    }

    Ok(())
}

pub fn validate_template_references(
    workflow: &Workflow,
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
    workflow: &Workflow,
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
                    reason: safe_identifier_message().to_string(),
                });
            }
            if !workflow.schemas.contains_key(items) {
                return Err(ValidationError::InvalidSchema {
                    schema: name.to_string(),
                    reason: format!("array.items references unknown schemas '{items}'"),
                });
            }
        }
        SchemaDef::String { r#enum } => {
            if r#enum.as_ref().is_some_and(Vec::is_empty) {
                return Err(ValidationError::InvalidSchema {
                    schema: name.to_string(),
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
    workflow: &Workflow,
) -> Result<(), ValidationError> {
    if let Some(contract) = artifact {
        validate_schema_reference_identifier(node_name, "artifact", contract)?;
        let schema =
            workflow
                .schemas
                .get(contract)
                .ok_or_else(|| ValidationError::UnknownSchemaRef {
                    step: node_name.to_string(),
                    slot: "artifact",
                    key: contract.to_string(),
                })?;
        if is_fanout || !matches!(schema, SchemaDef::Object { .. }) {
            return Err(ValidationError::InvalidArtifactSchema {
                step: node_name.to_string(),
                contract: contract.to_string(),
            });
        }
        if is_command {
            if let Some(field) = contract_schema::schema_declares_command_reserved_field(schema) {
                return Err(ValidationError::ReservedArtifactField {
                    step: node_name.to_string(),
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
                step: node_name.to_string(),
                slot: "input",
                key: contract.to_string(),
            });
        }
    }

    Ok(())
}

fn validate_schema_reference_identifier(
    step: &str,
    slot: &'static str,
    key: &str,
) -> Result<(), ValidationError> {
    if contract_schema::is_safe_identifier(key) {
        return Ok(());
    }
    Err(ValidationError::InvalidSchemaRef {
        step: step.to_string(),
        slot,
        key: key.to_string(),
        reason: safe_identifier_message().to_string(),
    })
}

fn safe_identifier_message() -> &'static str {
    "must start with an ASCII alphanumeric character and contain only ASCII alphanumeric characters, '-' or '_'"
}

/// node 数上限 (`MAX_NODES_PER_WORKFLOW`) と parallel 子 node 数上限
/// (`MAX_PARALLEL_CHILDREN`) の DoS ガードを評価する。
///
/// `TooManyNodes` を検出した時点で後続の per-step `TooManyParallelChildren`
/// 検査はスキップし、上限超過 1 件のみを返す（名前空間構築すら無意味な状態のため）。
/// `validate` / `validate_all` の両経路から呼ばれ、呼び出し側はそれぞれ
/// 「最初のエラーで return」「全件 push して以降のチェックを打ち切る」と
/// 消費方法を切り替える。
fn collect_node_count_errors(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let total_nodes = total_node_count(workflow);
    if total_nodes > MAX_NODES_PER_WORKFLOW {
        errors.push(ValidationError::TooManyNodes {
            count: total_nodes,
            max: MAX_NODES_PER_WORKFLOW,
        });
        return errors;
    }
    for step in &workflow.nodes {
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            if children.len() > MAX_PARALLEL_CHILDREN {
                errors.push(ValidationError::TooManyParallelChildren {
                    step: step.name.clone(),
                    count: children.len(),
                    max: MAX_PARALLEL_CHILDREN,
                });
            }
        }
    }
    errors
}

/// session (collect なし) に facet 参照が無い場合に
/// `MissingFacet` を返す。command node は `command` を持つため facet が
/// 不要であり、この検査の対象外（必須フィールドは `validate_node_kind_fields` で検証済み）。
///
/// `validate` / `validate_all` の両経路で同じ判定を行うため共通化する。
fn check_missing_facet(step: &NodeDefinition) -> Option<ValidationError> {
    if step.is_session() && step.collect.is_none() && !step.has_facet_refs() {
        Some(ValidationError::MissingFacet {
            step: step.name.clone(),
        })
    } else {
        None
    }
}

/// node kind ごとの許可フィールドを検証する。
fn validate_node_kind_fields(step: &NodeDefinition) -> Result<(), ValidationError> {
    let kind_name = step.kind_name().as_str();
    let disallow = |field: &'static str| ValidationError::DisallowedFieldForKind {
        step: step.name.clone(),
        field,
        kind: kind_name,
    };

    match step.kind_name() {
        NodeKindName::Command => {
            let command = step
                .command()
                .expect("command node must expose command spec");
            if command.trim().is_empty() {
                return Err(ValidationError::EmptyCommand {
                    step: step.name.clone(),
                });
            }
            if step.collect.is_some() {
                return Err(disallow("collect"));
            }
        }
        NodeKindName::Session => {}
        NodeKindName::Fanout => {
            if step.collect.is_some() {
                return Err(disallow("collect"));
            }
        }
    }
    Ok(())
}

/// ステップに permission が必須として指定されていることを検証する。
/// `None` または対象外の値（旧語彙・未知語彙・空文字）はバリデーションエラー。
fn validate_required_permission(
    step_name: &str,
    value: Option<&str>,
) -> Result<(), ValidationError> {
    match value {
        None => Err(ValidationError::MissingPermissionMode {
            step: step_name.to_string(),
        }),
        Some(v) => {
            if !is_allowed_permission_mode(v) {
                return Err(ValidationError::InvalidPermissionMode {
                    step: step_name.to_string(),
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

/// ワークフロー内の全ステップの `model` フィールドを検証する。
///
/// 検証は経路によらず同一の基準で行う:
/// 1. 形式検証（`crate::domain::agent_session::ModelId`）— 空文字・空白のみ・制御文字・
///    上限長超過は登録判定に進まず形式不正として拒否する
/// 2. 登録判定（呼び出し側の resolver）— 未登録なら `UnknownModel`
/// 3. 所属解決（呼び出し側の resolver）— 複数 backend に登録された曖昧な model は拒否する
pub fn validate_models<F>(workflow: &Workflow, mut resolve_model: F) -> Result<(), ValidationError>
where
    F: FnMut(&str) -> Result<Option<String>, String>,
{
    for step in &workflow.nodes {
        if let Some(model) = step.model() {
            validate_model_format(&step.name, model)?;
            validate_model_registered(&step.name, model, &mut resolve_model)?;
        }
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                if let Some(ref model) = child.model {
                    validate_model_format(&child.name, model)?;
                    validate_model_registered(&child.name, model, &mut resolve_model)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_model_format(step_name: &str, model: &str) -> Result<(), ValidationError> {
    crate::domain::agent_session::ModelId::parse(model).map_err(|reason| {
        ValidationError::InvalidModelFormat {
            step: step_name.to_string(),
            value: model.to_string(),
            reason,
        }
    })?;
    Ok(())
}

fn validate_model_registered<F>(
    step_name: &str,
    model: &str,
    resolve_model: &mut F,
) -> Result<(), ValidationError>
where
    F: FnMut(&str) -> Result<Option<String>, String>,
{
    match resolve_model(model) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ValidationError::UnknownModel {
            step: step_name.to_string(),
            value: model.to_string(),
        }),
        Err(reason) => Err(ValidationError::ModelResolutionFailed {
            step: step_name.to_string(),
            value: model.to_string(),
            reason,
        }),
    }
}

/// 診断用: 全てのバリデーションエラーを収集して返す。
/// `validate` は最初のエラーで早期リターンするが、診断エンジンでは全エラーを網羅的に報告したいため、
/// 構造的に安全な範囲でエラーを蓄積する。
/// 名前空間構築に失敗するレベルのエラー（EmptyName, EmptySteps, DuplicateStep等）は
/// 後続チェックが信頼できないため、そこで打ち切って返す。
pub fn validate_all(workflow: &Workflow) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if let Err(e) = validate_name(&workflow.name) {
        errors.push(e);
        return errors;
    }

    if workflow.nodes.is_empty() {
        errors.push(ValidationError::EmptySteps);
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

    // 名前空間構築: 重複があれば蓄積するが、以降のチェックは続行
    let mut transition_target_names = HashSet::new();
    let mut referenceable_step_names = HashSet::new();
    let mut has_dup = false;
    for step in &workflow.nodes {
        if !transition_target_names.insert(step.name.as_str()) {
            errors.push(ValidationError::DuplicateStep {
                name: step.name.clone(),
            });
            has_dup = true;
        }
        referenceable_step_names.insert(step.name.as_str());
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                if !referenceable_step_names.insert(child.name.as_str()) {
                    errors.push(ValidationError::ParallelChildNameConflict {
                        child: child.name.clone(),
                    });
                    has_dup = true;
                }
            }
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

    let mut preceding_step_names: HashSet<&str> = HashSet::new();

    for step in &workflow.nodes {
        if let Err(e) = validate_node_kind_fields(step) {
            errors.push(e);
        }
        if step.is_fanout() {
            let children = &step
                .fanout()
                .expect("is_fanout checked above")
                .parallel_children;
            if children.is_empty() {
                errors.push(ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "fanoutブロックには1つ以上の子ステップが必要です".to_string(),
                });
            }
            for child in children {
                if !child.has_facet_refs() {
                    errors.push(ValidationError::ParallelChildMissingFacet {
                        parent: step.name.clone(),
                        child: child.name.clone(),
                    });
                }
                if let Err(e) =
                    validate_required_permission(&child.name, child.permission.as_deref())
                {
                    errors.push(e);
                }
            }

            if let Some(agg) = step.fanout().and_then(|fanout| fanout.aggregate.as_ref()) {
                match (&agg.all_match, &agg.any_match) {
                    (Some(_), Some(_)) => {
                        errors.push(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: "all_matchとany_matchは同時に指定できません".to_string(),
                        });
                    }
                    (None, None) => {
                        errors.push(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: "all_matchまたはany_matchのいずれかが必要です".to_string(),
                        });
                    }
                    _ => {}
                }
                if let Some(ref pattern) = agg.all_match {
                    if RegexBuilder::new(pattern)
                        .size_limit(1 << 20)
                        .build()
                        .is_err()
                    {
                        errors.push(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: format!("all_matchのパターンが不正な正規表現です: {pattern}"),
                        });
                    }
                }
                if let Some(ref pattern) = agg.any_match {
                    if RegexBuilder::new(pattern)
                        .size_limit(1 << 20)
                        .build()
                        .is_err()
                    {
                        errors.push(ValidationError::AggregateInvalidConfig {
                            step: step.name.clone(),
                            reason: format!("any_matchのパターンが不正な正規表現です: {pattern}"),
                        });
                    }
                }
                if !transition_target_names.contains(agg.then.as_str()) {
                    errors.push(ValidationError::AggregateUnknownTarget {
                        step: step.name.clone(),
                        target: agg.then.clone(),
                    });
                }
                if !transition_target_names.contains(agg.r#else.as_str()) {
                    errors.push(ValidationError::AggregateUnknownTarget {
                        step: step.name.clone(),
                        target: agg.r#else.clone(),
                    });
                }
            }
        } else {
            // 新 schema では kind が型レベルで必須・列挙のため、旧 schema の
            // MissingMode / InteractiveModeNotAllowed 検査は YAML deserialize 段階で吸収される。
            if step.is_session() {
                if let Err(e) = validate_required_permission(&step.name, step.permission()) {
                    errors.push(e);
                }
            }
            if let Some(err) = check_missing_facet(step) {
                errors.push(err);
            }
            if let Some(ref collect) = step.collect {
                for r in &collect.from {
                    if !preceding_step_names.contains(r.as_str()) {
                        errors.push(ValidationError::UnknownCollectFrom {
                            step: step.name.clone(),
                            reference: r.clone(),
                        });
                    }
                }
            }
        }

        preceding_step_names.insert(&step.name);
        if let Some(fanout) = step.fanout() {
            let children = &fanout.parallel_children;
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        CollectConfig, CommandSpec, FacetRefs, FanoutSpec, InterimChild, NodeKind,
        ParallelAggregate, Rule, SchemaDef, SessionGate, SessionSpec,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Copy)]
    enum TestKind {
        Session,
        ApprovalSession,
    }

    fn make_workflow(nodes: Vec<NodeDefinition>) -> Workflow {
        Workflow {
            schemas: Default::default(),
            name: "test".to_string(),
            description: "test workflow".to_string(),
            builtin: false,
            nodes,
        }
    }

    fn resolve_from_set(valid: &HashSet<String>, model: &str) -> Result<Option<String>, String> {
        Ok(valid.contains(model).then(|| "backend".to_string()))
    }

    fn make_step(name: &str, kind: TestKind, rules: Vec<Rule>) -> NodeDefinition {
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
            rules: rules,
            ..NodeDefinition::default()
        }
    }

    fn make_parallel_step(name: &str) -> InterimChild {
        InterimChild {
            name: name.to_string(),
            facets: FacetRefs {
                instruction: Some("review".to_string()),
                ..Default::default()
            },
            permission: Some("edit".to_string()),
            ..InterimChild::default()
        }
    }

    fn make_parallel_block(
        name: &str,
        children: Vec<InterimChild>,
        aggregate: Option<ParallelAggregate>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                parallel_children: children,
                aggregate,
            }),
            ..NodeDefinition::default()
        }
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

    fn command_step(name: &str, command: &str) -> NodeDefinition {
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
    ) -> Workflow {
        Workflow {
            schemas,
            ..make_workflow(nodes)
        }
    }

    // ---- 既存テスト ----

    #[test]
    fn valid_workflow_passes() {
        let wf = make_workflow(vec![
            make_step("plan", TestKind::ApprovalSession, vec![]),
            NodeDefinition {
                rules: vec![
                    Rule::LoopGuard {
                        max_iterations: 3,
                        on_exhausted: "plan".to_string(),
                    },
                    Rule::Next("plan".to_string()),
                ],
                ..make_step("implement", TestKind::Session, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    // [02]: Interactive 概念が廃止されたため、
    // 旧テスト `interactive_mode_fails_validation` は削除した。

    #[test]
    fn approval_step_allows_terminal_rules_empty() {
        let wf = make_workflow(vec![
            make_step("fix", TestKind::Session, vec![]),
            make_step("approval", TestKind::ApprovalSession, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn rules_reject_multiple_next_catch_alls() {
        let wf = make_workflow(vec![
            make_step("fix", TestKind::Session, vec![]),
            make_step(
                "route",
                TestKind::ApprovalSession,
                vec![Rule::Next("fix".to_string()), Rule::Next("fix".to_string())],
            ),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidRules { ref step, .. } if step == "route"
        ));
    }

    #[test]
    fn rules_reject_standalone_next_with_discriminator() {
        let wf = make_workflow(vec![
            make_step("fix", TestKind::Session, vec![]),
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
                ..make_step("route", TestKind::Session, vec![])
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
            ValidationError::InvalidRules { ref step, .. } if step == "route"
        ));
    }

    #[test]
    fn invalid_transition_target_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            rules: vec![Rule::Next("nonexistent".to_string())],
            ..make_step("plan", TestKind::Session, vec![])
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
    fn routing_unknown_target_maps_by_variant_not_reason_text() {
        let err =
            routing_error_to_validation_error(routing::RoutingValidationError::UnknownRuleTarget {
                step: "route".to_string(),
                target: "missing".to_string(),
            });
        assert!(matches!(
            err,
            ValidationError::UnknownRuleTarget { ref step, ref target }
                if step == "route" && target == "missing"
        ));

        let err = routing_error_to_validation_error(routing::RoutingValidationError::Invalid {
            step: "route".to_string(),
            reason: "unknown rule target 'missing'".to_string(),
        });
        assert!(matches!(
            err,
            ValidationError::InvalidRules { ref step, ref reason }
                if step == "route" && reason.contains("unknown rule target")
        ));
    }

    #[test]
    fn empty_steps_fails() {
        let wf = make_workflow(vec![]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::EmptySteps
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
    fn duplicate_step_names_fails() {
        let wf = make_workflow(vec![
            make_step("plan", TestKind::ApprovalSession, vec![]),
            make_step("plan", TestKind::Session, vec![]),
        ]);
        let result = validate(&wf);
        assert!(matches!(
            result.unwrap_err(),
            ValidationError::DuplicateStep { ref name } if name == "plan"
        ));
    }

    #[test]
    fn missing_facet_without_collect_fails() {
        let wf = make_workflow(vec![without_session_facets(make_step(
            "step1",
            TestKind::Session,
            vec![],
        ))]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::MissingFacet { ref step } if step == "step1"
        ));
    }

    #[test]
    fn facet_only_step_passes() {
        let wf = make_workflow(vec![with_session_facets(
            make_step("step1", TestKind::Session, vec![]),
            FacetRefs {
                policy: Some("coding".to_string()),
                instruction: Some("implement".to_string()),
                ..Default::default()
            },
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn collect_step_without_facets_passes() {
        let wf = make_workflow(vec![
            make_step("review_a", TestKind::Session, vec![]),
            NodeDefinition {
                collect: Some(CollectConfig {
                    from: vec!["review_a".to_string()],
                    reduce: ReduceStrategy::Concat,
                }),
                ..without_session_facets(make_step("collect_reviews", TestKind::Session, vec![]))
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn reserved_artifact_names_cannot_be_nodes() {
        for name in ["request", "item"] {
            let wf = make_workflow(vec![make_step(name, TestKind::Session, vec![])]);
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
            ..make_step("step1", TestKind::Session, vec![])
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
            let mut plan = make_step("plan", TestKind::Session, vec![]);
            plan.artifact = Some("plan-doc".to_string());
            let wf = workflow_with_schemas(
                vec![
                    plan,
                    NodeDefinition {
                        inputs: vec![input.to_string()],
                        ..make_step("consume", TestKind::Session, vec![])
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
            ..make_step("consume", TestKind::Session, vec![])
        }]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason }
                if reference == "bad ref"
                    && reason == "`inputs:` entries must be `request` or a top-level node Artifact name"
        ));
    }

    #[test]
    fn invalid_template_reference_uses_template_context_reason() {
        let wf = make_workflow(vec![command_step("step1", "echo {{ bad ref }}")]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason }
                if reference == "bad ref"
                    && reason.contains("{{ ... }}")
                    && !reason.contains("inputs:")
        ));
    }

    #[test]
    fn validate_template_references_uses_template_context_reason() {
        let wf = make_workflow(vec![make_step("review", TestKind::Session, vec![])]);
        let errors = validate_template_references(&wf, "{{ bad ref }}", false);

        assert!(matches!(
            errors.as_slice(),
            [ValidationError::InvalidArtifactReference { reference, reason }]
                if reference == "bad ref"
                    && reason.contains("{{ ... }}")
                    && !reason.contains("inputs:")
        ));
    }

    #[test]
    fn legacy_task_template_reference_fails_when_no_artifact_exists() {
        let wf = make_workflow(vec![command_step("step1", "echo {{ task }}")]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. } if reference == "task"
        ));
    }

    #[test]
    fn item_template_reference_fails_outside_fanout_child_scope() {
        let wf = make_workflow(vec![command_step("step1", "echo {{ item.path }}")]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. } if reference == "item"
        ));
    }

    #[test]
    fn artifact_reference_to_session_without_artifact_fails() {
        let wf = make_workflow(vec![
            make_step("plan", TestKind::Session, vec![]),
            command_step("consume", "echo {{ plan }}"),
        ]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason }
                if reference == "plan" && reason.contains("does not produce")
        ));
    }

    #[test]
    fn command_without_artifact_rejects_non_reserved_field() {
        let wf = make_workflow(vec![
            command_step("build", "cargo build"),
            command_step("consume", "echo {{ build.no_such_field }}"),
        ]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason }
                if reference == "build.no_such_field" && reason.contains("unknown Artifact field")
        ));
    }

    #[test]
    fn artifact_node_rejects_undeclared_field() {
        let mut schemas = BTreeMap::new();
        schemas.insert("plan-doc".to_string(), artifact_object_schema(&["summary"]));
        let mut plan = make_step("plan", TestKind::Session, vec![]);
        plan.artifact = Some("plan-doc".to_string());
        let wf = workflow_with_schemas(
            vec![
                plan,
                command_step("consume", "echo {{ plan.unknown_field }}"),
            ],
            schemas,
        );

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason }
                if reference == "plan.unknown_field" && reason.contains("unknown Artifact field")
        ));
    }

    #[test]
    fn unknown_collect_from_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            collect: Some(CollectConfig {
                from: vec!["nonexistent".to_string()],
                reduce: ReduceStrategy::Concat,
            }),
            ..without_session_facets(make_step("step1", TestKind::Session, vec![]))
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownCollectFrom { ref reference, .. } if reference == "nonexistent"
        ));
    }

    #[test]
    fn valid_input_reference_passes() {
        let wf = make_workflow(vec![
            make_step("step_a", TestKind::Session, vec![]),
            NodeDefinition {
                ..make_step("step_b", TestKind::Session, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- 並列ブロック固有テスト ----

    #[test]
    fn valid_parallel_block_passes() {
        let wf = make_workflow(vec![
            make_step("implement", TestKind::Session, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(ParallelAggregate {
                    all_match: Some("LGTM".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_block_without_aggregate_passes() {
        let wf = make_workflow(vec![
            make_step("implement", TestKind::Session, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                None,
            ),
            make_step("report", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // 新 schema では node_type=Parallel が型レベルで mode を排除するため、
    // 旧テスト `parallel_block_with_mode_fails` は廃止された（[02] 範囲）。

    #[test]
    fn parallel_child_name_conflict_fails() {
        let wf = make_workflow(vec![
            make_step("conflict", TestKind::Session, vec![]),
            make_parallel_block("par", vec![make_parallel_step("conflict")], None),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ParallelChildNameConflict { ref child } if child == "conflict"
        ));
    }

    #[test]
    fn aggregate_both_match_fails() {
        let wf = make_workflow(vec![
            make_step("target", TestKind::Session, vec![]),
            make_parallel_block(
                "par",
                vec![make_parallel_step("child1")],
                Some(ParallelAggregate {
                    all_match: Some("LGTM".to_string()),
                    any_match: Some("NEEDS_FIX".to_string()),
                    then: "target".to_string(),
                    r#else: "target".to_string(),
                }),
            ),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::AggregateInvalidConfig { ref step, .. } if step == "par"
        ));
    }

    #[test]
    fn aggregate_no_match_fails() {
        let wf = make_workflow(vec![
            make_step("target", TestKind::Session, vec![]),
            make_parallel_block(
                "par",
                vec![make_parallel_step("child1")],
                Some(ParallelAggregate {
                    all_match: None,
                    any_match: None,
                    then: "target".to_string(),
                    r#else: "target".to_string(),
                }),
            ),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::AggregateInvalidConfig { ref step, .. } if step == "par"
        ));
    }

    #[test]
    fn aggregate_unknown_target_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![make_parallel_step("child1")],
            Some(ParallelAggregate {
                all_match: Some("LGTM".to_string()),
                any_match: None,
                then: "nonexistent".to_string(),
                r#else: "par".to_string(),
            }),
        )]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::AggregateUnknownTarget { ref target, .. } if target == "nonexistent"
        ));
    }

    #[test]
    fn fanout_inputs_are_rejected() {
        let mut fanout = make_parallel_block(
            "par",
            vec![make_parallel_step("child1"), make_parallel_step("child2")],
            None,
        );
        fanout.inputs = vec!["request".to_string()];
        let wf = make_workflow(vec![fanout]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, .. } if reference == "par"
        ));
    }

    #[test]
    fn parallel_child_missing_facet_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![InterimChild {
                facets: FacetRefs::default(),
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ParallelChildMissingFacet { ref parent, ref child }
                if parent == "par" && child == "child1"
        ));
    }

    // 新 schema では node_type が型レベルで必須となるため、旧テスト
    // `normal_step_missing_mode_fails` は YAML deserialize 段階で吸収される（[02] 範囲）。

    #[test]
    fn parallel_child_input_reference_valid_global_step() {
        let wf = make_workflow(vec![
            make_step("plan", TestKind::Session, vec![]),
            make_parallel_block(
                "par",
                vec![InterimChild {
                    ..make_parallel_step("child1")
                }],
                None,
            ),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn collect_from_parallel_children_passes() {
        let wf = make_workflow(vec![
            make_parallel_block(
                "par",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(ParallelAggregate {
                    all_match: Some("LGTM".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "report".to_string(),
                }),
            ),
            NodeDefinition {
                collect: Some(CollectConfig {
                    from: vec!["arch-review".to_string(), "security-review".to_string()],
                    reduce: ReduceStrategy::Concat,
                }),
                ..without_session_facets(make_step("report", TestKind::Session, vec![]))
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn empty_parallel_children_fails() {
        let wf = make_workflow(vec![
            make_step("implement", TestKind::Session, vec![]),
            make_parallel_block("parallel-review", vec![], None),
            make_step("report", TestKind::Session, vec![]),
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::AggregateInvalidConfig { ref reason, .. }
                if reason.contains("1つ以上の子ステップ")
        ));
    }

    #[test]
    fn invalid_regex_all_match_fails() {
        let wf = make_workflow(vec![
            make_step("implement", TestKind::Session, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(ParallelAggregate {
                    all_match: Some("[invalid(regex".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", TestKind::Session, vec![]),
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::AggregateInvalidConfig { ref reason, .. }
                if reason.contains("不正な正規表現")
        ));
    }

    #[test]
    fn invalid_regex_any_match_fails() {
        let wf = make_workflow(vec![
            make_step("implement", TestKind::Session, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(ParallelAggregate {
                    all_match: None,
                    any_match: Some("(unclosed".to_string()),
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", TestKind::Session, vec![]),
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::AggregateInvalidConfig { ref reason, .. }
                if reason.contains("不正な正規表現")
        ));
    }

    #[test]
    fn valid_regex_aggregate_passes() {
        let wf = make_workflow(vec![
            make_step("implement", TestKind::Session, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(ParallelAggregate {
                    all_match: Some(r"<decision>(LGTM|APPROVED)</decision>".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_input_reference_subsequent_step_passes() {
        // parallel block より後に定義されたステップへの後方参照は許可される
        // （出力が未生成の場合は空として扱われる）
        let wf = make_workflow(vec![
            make_step("plan", TestKind::Session, vec![]),
            make_parallel_block(
                "par",
                vec![InterimChild {
                    ..make_parallel_step("child1")
                }],
                None,
            ),
            make_step("report", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_input_reference_preceding_step_passes() {
        // parallel block より前に定義されたステップへの参照はOK
        let wf = make_workflow(vec![
            make_step("plan", TestKind::Session, vec![]),
            make_step("implement", TestKind::Session, vec![]),
            make_parallel_block(
                "par",
                vec![InterimChild {
                    ..make_parallel_step("child1")
                }],
                None,
            ),
            make_step("report", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_input_reference_prior_parallel_child_passes() {
        // 前のparallel blockの子stepへの参照はOK
        let wf = make_workflow(vec![
            make_parallel_block(
                "par1",
                vec![
                    make_parallel_step("review-a"),
                    make_parallel_step("review-b"),
                ],
                None,
            ),
            make_parallel_block(
                "par2",
                vec![InterimChild {
                    ..make_parallel_step("summarize")
                }],
                None,
            ),
        ]);
        assert!(validate(&wf).is_ok());
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
                ..make_step("fix", TestKind::Session, vec![])
            },
            make_step("approval", TestKind::ApprovalSession, vec![]),
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
            ..make_step("fix", TestKind::Session, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownRuleTarget { ref step, ref target }
                if step == "fix" && target == "nonexistent"
        ));
    }

    #[test]
    fn cycle_without_reachable_loop_guard_fails() {
        let wf = make_workflow(vec![
            make_step(
                "step_a",
                TestKind::Session,
                vec![Rule::Next("step_b".to_string())],
            ),
            make_step(
                "step_b",
                TestKind::Session,
                vec![Rule::Next("step_a".to_string())],
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
            make_step(
                "step_a",
                TestKind::Session,
                vec![Rule::Next("step_b".to_string())],
            ),
            make_step(
                "step_b",
                TestKind::Session,
                vec![
                    Rule::LoopGuard {
                        max_iterations: 2,
                        on_exhausted: "done".to_string(),
                    },
                    Rule::Next("step_a".to_string()),
                ],
            ),
            make_step("done", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- input_reference 後方参照 ----

    #[test]
    fn input_reference_backward_reference_passes() {
        // 定義順で後方のステップを input_reference で参照できる
        let wf = make_workflow(vec![
            NodeDefinition {
                ..make_step("step_a", TestKind::Session, vec![])
            },
            make_step("step_b", TestKind::Session, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- permission バリデーション ----

    #[test]
    fn valid_permission_ask_passes() {
        let wf = make_workflow(vec![with_session_permission(
            make_step("step1", TestKind::Session, vec![]),
            Some("ask"),
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn valid_permission_edit_passes() {
        let wf = make_workflow(vec![with_session_permission(
            make_step("step1", TestKind::Session, vec![]),
            Some("edit"),
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn valid_permission_full_passes() {
        let wf = make_workflow(vec![with_session_permission(
            make_step("step1", TestKind::Session, vec![]),
            Some("full"),
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn legacy_permission_accept_edits_rejected() {
        for legacy in ["acceptEdits", "bypassPermissions", "plan", "default"] {
            let wf = make_workflow(vec![with_session_permission(
                make_step("step1", TestKind::Session, vec![]),
                Some(legacy),
            )]);
            let err = validate(&wf).unwrap_err();
            assert!(matches!(
                err,
                ValidationError::InvalidPermissionMode { ref step, ref value }
                    if step == "step1" && value == legacy
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
            make_step("step1", TestKind::Session, vec![]),
            Some("invalid-mode"),
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref step, ref value }
                if step == "step1" && value == "invalid-mode"
        ));
        assert!(err.to_string().contains("ask, edit, full"));
    }

    #[test]
    fn empty_permission_fails() {
        let wf = make_workflow(vec![with_session_permission(
            make_step("step1", TestKind::Session, vec![]),
            Some(""),
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref step, ref value }
                if step == "step1" && value.is_empty()
        ));
        assert!(err.to_string().contains("ask, edit, full"));
    }

    #[test]
    fn invalid_permission_on_parallel_child_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![InterimChild {
                permission: Some("acceptEdits".to_string()),
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref step, ref value }
                if step == "child1" && value == "acceptEdits"
        ));
    }

    #[test]
    fn valid_permission_on_parallel_child_passes() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![InterimChild {
                permission: Some("full".to_string()),
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn step_without_permission_fails() {
        let wf = make_workflow(vec![with_session_permission(
            make_step("step1", TestKind::Session, vec![]),
            None,
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MissingPermissionMode { ref step } if step == "step1"
        ));
        assert!(err.to_string().contains("ask, edit, full"));
    }

    #[test]
    fn parallel_child_without_permission_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![InterimChild {
                permission: None,
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MissingPermissionMode { ref step } if step == "child1"
        ));
    }

    #[test]
    fn parallel_block_without_permission_passes_when_children_have_permission() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![InterimChild {
                permission: Some("edit".to_string()),
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        assert!(validate(&wf).is_ok());
    }

    // ---- model バリデーション (validate_models) ----

    #[test]
    fn validate_models_valid_model_passes() {
        let wf = make_workflow(vec![with_session_model(
            make_step("step1", TestKind::Session, vec![]),
            Some("haiku"),
        )]);
        let valid = HashSet::from(["haiku".to_string(), "opus-4".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_unknown_model_fails() {
        let wf = make_workflow(vec![with_session_model(
            make_step("step1", TestKind::Session, vec![]),
            Some("unknown-model"),
        )]);
        let valid = HashSet::from(["haiku".to_string(), "opus-4".to_string()]);
        let err = validate_models(&wf, |model| resolve_from_set(&valid, model)).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownModel { ref step, ref value }
                if step == "step1" && value == "unknown-model"
        ));
        assert!(err.to_string().contains("unknown model: unknown-model"));
    }

    #[test]
    fn validate_models_unknown_model_on_parallel_child_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![InterimChild {
                model: Some("unknown-model".to_string()),
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        let valid = HashSet::from(["haiku".to_string()]);
        let err = validate_models(&wf, |model| resolve_from_set(&valid, model)).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownModel { ref step, ref value }
                if step == "child1" && value == "unknown-model"
        ));
    }

    #[test]
    fn validate_models_rejects_ambiguous_model_from_resolver() {
        let wf = make_workflow(vec![with_session_model(
            make_step("step1", TestKind::Session, vec![]),
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
            ValidationError::ModelResolutionFailed { ref step, ref value, ref reason }
                if step == "step1" && value == "shared" && reason.contains("複数")
        ));
    }

    #[test]
    fn validate_models_no_model_specified_passes() {
        let wf = make_workflow(vec![make_step("step1", TestKind::Session, vec![])]);
        let valid = HashSet::from(["haiku".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_valid_model_on_parallel_child_passes() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![InterimChild {
                model: Some("haiku".to_string()),
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        let valid = HashSet::from(["haiku".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_rejects_empty_model_before_registry_check() {
        // 形式不正（空文字）は registry に含まれるかにかかわらず拒否される。
        // 未登録（UnknownModel）と区別するため InvalidModelFormat として報告される。
        let wf = make_workflow(vec![with_session_model(
            make_step("step1", TestKind::Session, vec![]),
            Some(""),
        )]);
        // valid_models に空文字を含めても形式検証で先に弾く
        let valid = HashSet::from([String::new(), "haiku".to_string()]);
        let err = validate_models(&wf, |model| resolve_from_set(&valid, model)).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidModelFormat { ref step, ref value, .. }
                if step == "step1" && value.is_empty()
        ));
    }

    #[test]
    fn validate_models_rejects_whitespace_only_model() {
        let wf = make_workflow(vec![with_session_model(
            make_step("step1", TestKind::Session, vec![]),
            Some("   "),
        )]);
        let valid = HashSet::from(["   ".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_err());
    }

    #[test]
    fn validate_models_rejects_control_character_model() {
        let wf = make_workflow(vec![with_session_model(
            make_step("step1", TestKind::Session, vec![]),
            Some("a\u{0001}b"),
        )]);
        let valid = HashSet::from(["a\u{0001}b".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_err());
    }

    // ---- command kind validation ----

    #[test]
    fn command_node_with_command_passes_when_facets_absent() {
        let wf = make_workflow(vec![command_step("build", "cargo build")]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn command_node_with_empty_command_fails() {
        let wf = make_workflow(vec![command_step("build", "   ")]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::EmptyCommand { ref step } if step == "build"
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
        let mut session = make_step("review", TestKind::Session, vec![]);
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
        let mut request_wf = make_workflow(vec![make_step("review", TestKind::Session, vec![])]);
        request_wf
            .schemas
            .insert("request".to_string(), SchemaDef::String { r#enum: None });
        assert!(matches!(
            validate_schema_refs(&request_wf).unwrap_err(),
            ValidationError::InvalidArtifactReference { ref reference, ref reason }
                if reference == "request" && reason.contains("reserved Artifact name")
        ));

        let mut item_wf = make_workflow(vec![make_step("review", TestKind::Session, vec![])]);
        item_wf
            .schemas
            .insert("item".to_string(), SchemaDef::String { r#enum: None });
        assert!(validate_schema_refs(&item_wf).is_ok());
    }

    #[test]
    fn schema_refs_reject_unknown_artifact_schema() {
        let mut session = make_step("review", TestKind::Session, vec![]);
        session.artifact = Some("missing".to_string());
        let wf = make_workflow(vec![session]);

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownSchemaRef { ref step, slot, ref key }
                if step == "review" && slot == "artifact" && key == "missing"
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_schema_identifier() {
        let mut session = make_step("review", TestKind::Session, vec![]);
        session.artifact = Some("review; curl https://example.invalid #".to_string());
        let mut wf = make_workflow(vec![session]);
        wf.schemas.insert(
            "review; curl https://example.invalid #".to_string(),
            object_schema(&["status"]),
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason }
                if schema == "review; curl https://example.invalid #"
                    && reason.contains("must start with an ASCII alphanumeric")
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_artifact_reference_identifier() {
        let mut session = make_step("review", TestKind::Session, vec![]);
        session.artifact = Some("review; curl https://example.invalid #".to_string());
        let wf = make_workflow(vec![session]);

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchemaRef { ref step, slot, ref key, ref reason }
                if step == "review"
                    && slot == "artifact"
                    && key == "review; curl https://example.invalid #"
                    && reason.contains("must start with an ASCII alphanumeric")
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_input_reference_identifier() {
        let mut session = make_step("review", TestKind::Session, vec![]);
        session.input = Some("../outside".to_string());
        let wf = make_workflow(vec![session]);

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchemaRef { ref step, slot, ref key, .. }
                if step == "review" && slot == "input" && key == "../outside"
        ));
    }

    #[test]
    fn schema_refs_reject_invalid_array_items_identifier() {
        let mut wf = make_workflow(vec![make_step("review", TestKind::Session, vec![])]);
        wf.schemas.insert(
            "review-list".to_string(),
            SchemaDef::Array {
                items: "../outside".to_string(),
            },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason }
                if schema == "review-list"
                    && reason.contains("must start with an ASCII alphanumeric")
        ));
    }

    #[test]
    fn schema_refs_reject_required_field_missing_from_properties() {
        let mut wf = make_workflow(vec![make_step("review", TestKind::Session, vec![])]);
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
            ValidationError::InvalidSchema { ref schema, ref reason }
                if schema == "review-output"
                    && reason == "required field 'verdict' is not declared in properties"
        ));
    }

    #[test]
    fn schema_refs_reject_empty_string_enum() {
        let mut wf = make_workflow(vec![make_step("review", TestKind::Session, vec![])]);
        wf.schemas.insert(
            "review-output".to_string(),
            SchemaDef::String {
                r#enum: Some(Vec::new()),
            },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason }
                if schema == "review-output" && reason == "enum must contain at least one value"
        ));
    }

    #[test]
    fn schema_refs_reject_array_items_unknown_schema() {
        let mut wf = make_workflow(vec![make_step("review", TestKind::Session, vec![])]);
        wf.schemas.insert(
            "review-list".to_string(),
            SchemaDef::Array {
                items: "missing-item".to_string(),
            },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidSchema { ref schema, ref reason }
                if schema == "review-list"
                    && reason == "array.items references unknown schemas 'missing-item'"
        ));
    }

    #[test]
    fn schema_refs_reject_session_artifact_non_object_schema() {
        let mut session = make_step("review", TestKind::Session, vec![]);
        session.artifact = Some("review-output".to_string());
        let mut wf = make_workflow(vec![session]);
        wf.schemas.insert(
            "review-output".to_string(),
            SchemaDef::String { r#enum: None },
        );

        assert!(matches!(
            validate_schema_refs(&wf).unwrap_err(),
            ValidationError::InvalidArtifactSchema { ref step, ref contract }
                if step == "review" && contract == "review-output"
        ));
    }

    #[test]
    fn fanout_node_rejects_artifact_declaration() {
        let mut fanout = make_parallel_block("review", vec![make_parallel_step("review-a")], None);
        fanout.artifact = Some("review-output".to_string());
        let mut wf = make_workflow(vec![fanout]);
        wf.schemas
            .insert("review-output".to_string(), object_schema(&["status"]));

        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidArtifactSchema { ref step, ref contract }
                if step == "review" && contract == "review-output"
        ));
    }

    #[test]
    fn command_node_rejects_artifact_reserved_field_collision() {
        let mut command = command_step("build", "cargo build");
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
            ValidationError::ReservedArtifactField { ref step, ref contract, ref field }
                if step == "build" && contract == "build-output" && field == "ok"
        ));
    }

    // ---- DoS ガードのテスト ----

    #[test]
    fn too_many_nodes_fails() {
        let nodes: Vec<NodeDefinition> = (0..MAX_NODES_PER_WORKFLOW + 1)
            .map(|i| make_step(&format!("step{i}"), TestKind::Session, vec![]))
            .collect();
        let wf = make_workflow(nodes);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::TooManyNodes { .. }
        ));
    }

    /// [02] DoS ガード: top-level + parallel_children を合算した総 node 数で上限を判定する。
    #[test]
    fn too_many_nodes_counts_parallel_children() {
        // top-level: parallel block 1 + agent 1 = 2 nodes。
        // parallel_children を MAX_NODES_PER_WORKFLOW 個に積むことで合算で上限超過する。
        let children: Vec<InterimChild> = (0..MAX_NODES_PER_WORKFLOW)
            .take(MAX_PARALLEL_CHILDREN)
            .map(|i| make_parallel_step(&format!("c{i}")))
            .collect();
        // children は MAX_PARALLEL_CHILDREN にクリップされる。
        // ガードを越えるため、複数 parallel block で children を積み増す。
        let need_blocks = (MAX_NODES_PER_WORKFLOW + 1).div_ceil(MAX_PARALLEL_CHILDREN) + 1;
        let mut nodes: Vec<NodeDefinition> = Vec::new();
        for b in 0..need_blocks {
            let block_children: Vec<InterimChild> = children
                .iter()
                .enumerate()
                .map(|(i, _)| make_parallel_step(&format!("blk{b}c{i}")))
                .collect();
            nodes.push(make_parallel_block(
                &format!("par{b}"),
                block_children,
                Some(ParallelAggregate {
                    all_match: Some("LGTM".to_string()),
                    any_match: None,
                    then: "par0".to_string(),
                    r#else: "par0".to_string(),
                }),
            ));
        }
        let wf = make_workflow(nodes);
        let total = total_node_count(&wf);
        assert!(
            total > MAX_NODES_PER_WORKFLOW,
            "test setup: total {total} must exceed limit"
        );
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::TooManyNodes { .. }
        ));
    }

    #[test]
    fn too_many_parallel_children_fails() {
        let children: Vec<InterimChild> = (0..MAX_PARALLEL_CHILDREN + 1)
            .map(|i| make_parallel_step(&format!("c{i}")))
            .collect();
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            children,
            Some(ParallelAggregate {
                all_match: Some("LGTM".to_string()),
                any_match: None,
                then: "par".to_string(),
                r#else: "par".to_string(),
            }),
        )]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::TooManyParallelChildren { ref step, .. } if step == "par"
        ));
    }

    #[test]
    fn validate_schema_refs_inspects_parallel_children() {
        let child = InterimChild {
            input: Some("nope".to_string()),
            ..make_parallel_step("child1")
        };
        let par = make_parallel_block(
            "par",
            vec![child],
            Some(ParallelAggregate {
                all_match: Some("LGTM".to_string()),
                any_match: None,
                then: "par".to_string(),
                r#else: "par".to_string(),
            }),
        );
        let wf = make_workflow(vec![par]);
        let err = validate_schema_refs(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownSchemaRef { ref step, slot, ref key }
                if step == "child1" && slot == "input" && key == "nope"
        ));
    }
}
