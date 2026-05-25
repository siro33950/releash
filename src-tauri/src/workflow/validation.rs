use super::schema::{NodeType, Workflow, MAX_NODES_PER_WORKFLOW, MAX_PARALLEL_CHILDREN};
use crate::permission::PermissionMode;
use regex::RegexBuilder;
use serde::Serialize;
use std::collections::HashSet;
use std::fmt;

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
    UnknownNextStep {
        step: String,
        next: String,
    },
    MissingFacet {
        step: String,
    },
    UnknownOutputFrom {
        step: String,
        reference: String,
    },
    UnknownCollectFrom {
        step: String,
        reference: String,
    },
    /// 並列子stepが agent 以外の node_type
    ParallelChildNotAuto {
        parent: String,
        child: String,
    },
    /// 並列子step名がグローバル名前空間で重複
    ParallelChildNameConflict {
        child: String,
    },
    /// aggregate が parallel なしで指定されている
    AggregateWithoutParallel {
        step: String,
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
    /// 並列子stepが同一block内の兄弟stepを pass_output_from で参照
    ParallelChildSiblingRef {
        parent: String,
        child: String,
        reference: String,
    },
    /// 並列子stepにファセット参照がない
    ParallelChildMissingFacet {
        parent: String,
        child: String,
    },
    /// on_exhausted が存在しないステップを参照
    UnknownOnExhausted {
        step: String,
        target: String,
    },
    /// resets_cycle_for が存在しないステップを参照
    UnknownResetsCycleFor {
        step: String,
        target: String,
    },
    /// on_exhausted の遷移チェーンが循環を形成
    CircularOnExhausted {
        cycle: Vec<String>,
    },
    /// resets_cycle_for が cycle_guard を持たないステップを参照
    ResetsCycleForNonGuardedStep {
        step: String,
        target: String,
    },
    /// approval step の rules は最大1件の match: reject のみ許可
    InvalidApprovalRules {
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
    /// bash 種別 node に `command` が指定されていない
    MissingCommand {
        step: String,
    },
    /// bash 種別 node の `command` が空文字
    EmptyCommand {
        step: String,
    },
    /// node 種別ごとに許可されないフィールドが指定されている
    DisallowedFieldForNodeType {
        step: String,
        field: &'static str,
        node_type: &'static str,
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
    /// `input_contracts` / `output_contract` が存在しない Contract facet を参照している。
    /// 信頼境界外入力 (user-authored workflow / フロントエンド編集) の保存時に検出する。
    UnknownContractRef {
        step: String,
        slot: &'static str,
        key: String,
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
            Self::UnknownNextStep { step, next } => write!(
                f,
                "ステップ '{step}' のルールが存在しないステップ '{next}' を参照しています"
            ),
            Self::MissingFacet { step } => {
                write!(
                    f,
                    "ステップ '{step}' にはファセット参照またはinline_promptが必要です（collectステップのみ両方省略可）"
                )
            }
            Self::UnknownOutputFrom { step, reference } => write!(
                f,
                "ステップ '{step}' のpass_output_fromが存在しないステップ '{reference}' を参照しています"
            ),
            Self::UnknownCollectFrom { step, reference } => write!(
                f,
                "ステップ '{step}' のcollect.fromが存在しないステップ '{reference}' を参照しています"
            ),
            Self::ParallelChildNotAuto { parent, child } => {
                write!(
                    f,
                    "parallelブロック '{parent}' の子node '{child}' は `type: agent` のみ許可されています"
                )
            }
            Self::ParallelChildNameConflict { child } => {
                write!(f, "並列子ステップ名 '{child}' が他のステップ名と重複しています")
            }
            Self::AggregateWithoutParallel { step } => {
                write!(
                    f,
                    "ステップ '{step}' にaggregateが定義されていますがparallelがありません"
                )
            }
            Self::AggregateInvalidConfig { step, reason } => {
                write!(f, "ステップ '{step}' のaggregate設定が不正です: {reason}")
            }
            Self::AggregateUnknownTarget { step, target } => write!(
                f,
                "ステップ '{step}' のaggregateが存在しないステップ '{target}' を参照しています"
            ),
            Self::ParallelChildSiblingRef {
                parent,
                child,
                reference,
            } => write!(
                f,
                "parallelブロック '{parent}' の子ステップ '{child}' のpass_output_fromが同一ブロック内の兄弟ステップ '{reference}' を参照しています"
            ),
            Self::ParallelChildMissingFacet { parent, child } => write!(
                f,
                "parallelブロック '{parent}' の子ステップ '{child}' にはファセット参照が必要です"
            ),
            Self::UnknownOnExhausted { step, target } => write!(
                f,
                "ステップ '{step}' のon_exhaustedが存在しないステップ '{target}' を参照しています"
            ),
            Self::UnknownResetsCycleFor { step, target } => write!(
                f,
                "ステップ '{step}' のresets_cycle_forが存在しないステップ '{target}' を参照しています"
            ),
            Self::CircularOnExhausted { cycle } => write!(
                f,
                "on_exhaustedの遷移チェーンが循環しています: {}",
                cycle.join(" → ")
            ),
            Self::ResetsCycleForNonGuardedStep { step, target } => write!(
                f,
                "ステップ '{step}' のresets_cycle_forがcycle_guardを持たないステップ '{target}' を参照しています"
            ),
            Self::InvalidApprovalRules { step, reason } => {
                write!(f, "approvalステップ '{step}' のrulesが不正です: {reason}")
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
                    PermissionMode::allowed_list()
                )
            }
            Self::MissingPermissionMode { step } => {
                write!(
                    f,
                    "ステップ '{step}' にはpermissionが必要です (allowed: {})",
                    PermissionMode::allowed_list()
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
            Self::MissingCommand { step } => {
                write!(
                    f,
                    "bashステップ '{step}' には command が必要です"
                )
            }
            Self::EmptyCommand { step } => {
                write!(
                    f,
                    "bashステップ '{step}' の command は空にできません"
                )
            }
            Self::DisallowedFieldForNodeType {
                step,
                field,
                node_type,
            } => write!(
                f,
                "ステップ '{step}' ({node_type}) には '{field}' を指定できません"
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
            Self::UnknownContractRef { step, slot, key } => {
                write!(
                    f,
                    "ステップ '{step}' の {slot} が存在しない Contract facet '{key}' を参照しています"
                )
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl Serialize for ValidationError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::EmptyName);
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err(ValidationError::InvalidChars {
            name: name.to_string(),
        });
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(ValidationError::InvalidChars {
            name: name.to_string(),
        });
    }
    Ok(())
}

/// `workflow.nodes` の top-level node 数と全 `parallel_children` の合算（=DoS ガード対象の総 node 数）。
fn total_node_count(workflow: &Workflow) -> usize {
    workflow.nodes.iter().fold(0usize, |acc, n| {
        let child_count = n.parallel_children.as_ref().map(|c| c.len()).unwrap_or(0);
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

    // 遷移先名前空間: トップレベルstep名のみ（aggregate.then/else, rule.nextの検証用）
    let mut transition_target_names = HashSet::new();
    // 参照可能名前空間: トップレベルstep名 + 並列子step名（pass_output_from等の検証用）
    let mut referenceable_step_names = HashSet::new();
    for step in &workflow.nodes {
        if !transition_target_names.insert(step.name.as_str()) {
            return Err(ValidationError::DuplicateStep {
                name: step.name.clone(),
            });
        }
        referenceable_step_names.insert(step.name.as_str());
        // 並列子step名は参照可能名前空間にのみ追加（遷移先には不可）
        if let Some(ref children) = step.parallel_children {
            for child in children {
                if !referenceable_step_names.insert(child.name.as_str()) {
                    return Err(ValidationError::ParallelChildNameConflict {
                        child: child.name.clone(),
                    });
                }
            }
        }
    }

    // 各ステップより前に定義されたステップ名を追跡（parallel子stepのpass_output_from検証用）
    let mut preceding_step_names: HashSet<&str> = HashSet::new();

    for step in &workflow.nodes {
        validate_node_type_fields(step)?;
        if step.is_parallel() {
            // --- parallel block 固有のバリデーション ---
            // [02] では node_type=Parallel が型レベルで mode を排除するため、
            // 旧 schema の「parallel に mode 指定」エラーは存在しない。

            let children = step.parallel_children.as_ref().ok_or_else(|| {
                ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "parallelブロックには parallel_children が必要です".to_string(),
                }
            })?;
            if children.is_empty() {
                return Err(ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "parallelブロックには1つ以上の子ステップが必要です".to_string(),
                });
            }
            let child_names: HashSet<&str> = children.iter().map(|c| c.name.as_str()).collect();

            for child in children {
                // 子step は auto のみ
                if child.node_type != NodeType::Agent {
                    return Err(ValidationError::ParallelChildNotAuto {
                        parent: step.name.clone(),
                        child: child.name.clone(),
                    });
                }

                // 子step にはファセット参照が必要
                if !child.has_facet_refs() {
                    return Err(ValidationError::ParallelChildMissingFacet {
                        parent: step.name.clone(),
                        child: child.name.clone(),
                    });
                }

                // 子step の permission 妥当性チェック（必須）
                validate_required_permission(&child.name, child.permission.as_deref())?;

                // pass_output_from の参照先チェック
                if let Some(ref refs) = child.pass_output_from {
                    for r in refs {
                        // 同一block内の兄弟子step参照は禁止
                        if child_names.contains(r.as_str()) {
                            return Err(ValidationError::ParallelChildSiblingRef {
                                parent: step.name.clone(),
                                child: child.name.clone(),
                                reference: r.clone(),
                            });
                        }
                        // 定義済みステップ（兄弟以外）を参照可能（後方参照も許可）
                        if !referenceable_step_names.contains(r.as_str()) {
                            return Err(ValidationError::UnknownOutputFrom {
                                step: child.name.clone(),
                                reference: r.clone(),
                            });
                        }
                    }
                }
            }

            // aggregate バリデーション
            if let Some(ref agg) = step.aggregate {
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
            // 新 schema では node_type が型レベルで必須・列挙のため、
            // 旧 schema の MissingMode / InteractiveModeNotAllowed 検査は
            // YAML deserialize 段階で吸収される（[02] 範囲外）。
            if step.node_type == NodeType::Approval {
                validate_approval_rules(&step.name, &step.transition_rules)?;
            }

            // permission の妥当性チェック（必須）
            validate_required_permission(&step.name, step.permission.as_deref())?;

            // aggregate が parallel なしで指定されている場合はエラー
            if step.aggregate.is_some() {
                return Err(ValidationError::AggregateWithoutParallel {
                    step: step.name.clone(),
                });
            }

            if let Some(err) = check_missing_facet(step) {
                return Err(err);
            }

            // rules の遷移先チェック（トップレベルstepのみ）
            for rule in &step.transition_rules {
                if !transition_target_names.contains(rule.next.as_str()) {
                    return Err(ValidationError::UnknownNextStep {
                        step: step.name.clone(),
                        next: rule.next.clone(),
                    });
                }
            }

            // pass_output_from の参照先 step 名が定義済みステップに存在するか検証
            // （後方参照を許可：出力が未生成の場合は空として扱われる）
            if let Some(ref refs) = step.pass_output_from {
                for r in refs {
                    if !referenceable_step_names.contains(r.as_str()) {
                        return Err(ValidationError::UnknownOutputFrom {
                            step: step.name.clone(),
                            reference: r.clone(),
                        });
                    }
                }
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
                    super::schema::ReduceStrategy::AnyNeedsFix
                        | super::schema::ReduceStrategy::AllPassed
                ) {
                    for r in &collect.from {
                        let referenced_step = workflow.nodes.iter().find(|s| s.name == *r);
                        if let Some(rs) = referenced_step {
                            if rs.transition_rules.is_empty() && !rs.is_parallel() {
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

        // on_exhausted の参照先検証
        if let Some(ref guard) = step.cycle_guard {
            if let Some(ref target) = guard.on_exhausted {
                if !transition_target_names.contains(target.as_str()) {
                    return Err(ValidationError::UnknownOnExhausted {
                        step: step.name.clone(),
                        target: target.clone(),
                    });
                }
            }
        }

        // resets_cycle_for の参照先検証
        if let Some(ref targets) = step.resets_cycle_for {
            for target in targets {
                if !transition_target_names.contains(target.as_str()) {
                    return Err(ValidationError::UnknownResetsCycleFor {
                        step: step.name.clone(),
                        target: target.clone(),
                    });
                }
                // 参照先が cycle_guard を持つか検証
                let target_step = workflow.nodes.iter().find(|s| s.name == *target);
                if let Some(ts) = target_step {
                    if ts.cycle_guard.is_none() {
                        return Err(ValidationError::ResetsCycleForNonGuardedStep {
                            step: step.name.clone(),
                            target: target.clone(),
                        });
                    }
                }
            }
        }

        // 次のステップのvalidationで使えるよう、このステップ名を追加
        preceding_step_names.insert(&step.name);
        // parallel blockの子step名も追加（後続parallel blockの子stepから参照可能にするため）
        if let Some(ref children) = step.parallel_children {
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }

    // on_exhausted の循環参照検出
    for step in &workflow.nodes {
        if let Some(ref guard) = step.cycle_guard {
            if let Some(ref target) = guard.on_exhausted {
                let mut visited = vec![step.name.clone()];
                let mut current = target.clone();
                loop {
                    if visited.contains(&current) {
                        visited.push(current);
                        return Err(ValidationError::CircularOnExhausted { cycle: visited });
                    }
                    visited.push(current.clone());
                    // current のステップの on_exhausted を辿る
                    let next = workflow
                        .nodes
                        .iter()
                        .find(|s| s.name == current)
                        .and_then(|s| s.cycle_guard.as_ref())
                        .and_then(|g| g.on_exhausted.as_ref());
                    match next {
                        Some(n) => current = n.clone(),
                        None => break,
                    }
                }
            }
        }
    }

    Ok(())
}

/// `input_contracts` / `output_contract` の参照キーが Contract facet として
/// 実在するかを検証する（[02] Contract 双方向対称性 + 信頼境界外入力の参照妥当性検査）。
///
/// `contract_exists` は呼び出し側が「facet base dir + builtin」での解決可否を返すクロージャ。
/// validation.rs は facet I/O を持たない（境界保持）ため、storage.rs などの呼び出し側で
/// `facet::load_facet(FacetKind::Contract, key, base_dir).is_ok()` を渡す形にする。
///
/// top-level node と parallel child の両方を網羅して検査する。
pub fn validate_facet_refs<F>(
    workflow: &Workflow,
    contract_exists: F,
) -> Result<(), ValidationError>
where
    F: Fn(&str) -> bool,
{
    fn check<F: Fn(&str) -> bool>(
        step_name: &str,
        output_contract: Option<&str>,
        input_contracts: Option<&[String]>,
        contract_exists: &F,
    ) -> Result<(), ValidationError> {
        if let Some(key) = output_contract {
            if !contract_exists(key) {
                return Err(ValidationError::UnknownContractRef {
                    step: step_name.to_string(),
                    slot: "output_contract",
                    key: key.to_string(),
                });
            }
        }
        if let Some(keys) = input_contracts {
            for key in keys {
                if !contract_exists(key) {
                    return Err(ValidationError::UnknownContractRef {
                        step: step_name.to_string(),
                        slot: "input_contracts",
                        key: key.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    for node in &workflow.nodes {
        check(
            &node.name,
            node.output_contract.as_deref(),
            node.input_contracts.as_deref(),
            &contract_exists,
        )?;
        if let Some(children) = node.parallel_children.as_ref() {
            for child in children {
                check(
                    &child.name,
                    child.output_contract.as_deref(),
                    child.input_contracts.as_deref(),
                    &contract_exists,
                )?;
            }
        }
    }
    Ok(())
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
        if let Some(ref children) = step.parallel_children {
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

/// agent/approval step (collect なし) に facet 参照も inline_prompt も無い場合に
/// `MissingFacet` を返す。bash step は `command` を持つため facet/inline_prompt が
/// 不要であり、この検査の対象外（必須フィールドは `validate_node_type_fields` で検証済み）。
///
/// `validate` / `validate_all` の両経路で同じ判定を行うため共通化する。
fn check_missing_facet(step: &super::schema::NodeDefinition) -> Option<ValidationError> {
    if step.node_type != NodeType::Bash
        && step.collect.is_none()
        && !step.has_facet_refs()
        && step.inline_prompt.is_none()
    {
        Some(ValidationError::MissingFacet {
            step: step.name.clone(),
        })
    } else {
        None
    }
}

/// node 種別ごとの許可フィールドを検証する（[02] schema 境界）。
///
/// - `Bash`: `command` 必須・空文字禁止。`facet refs` / `inline_prompt` /
///   `collect` / `parallel_children` / `aggregate` は不許可。
/// - `Agent` / `Approval`: `command` / `parallel_children` / `aggregate` は不許可。
/// - `Parallel`: `command` / `facet refs` / `inline_prompt` / `collect` は不許可
///   （`parallel_children` 自体は required で別経路の `AggregateInvalidConfig` で検証）。
fn validate_node_type_fields(step: &super::schema::NodeDefinition) -> Result<(), ValidationError> {
    let node_type_name = match step.node_type {
        NodeType::Agent => "agent",
        NodeType::Bash => "bash",
        NodeType::Approval => "approval",
        NodeType::Parallel => "parallel",
    };
    let disallow = |field: &'static str| ValidationError::DisallowedFieldForNodeType {
        step: step.name.clone(),
        field,
        node_type: node_type_name,
    };

    match step.node_type {
        NodeType::Bash => {
            let command =
                step.command
                    .as_deref()
                    .ok_or_else(|| ValidationError::MissingCommand {
                        step: step.name.clone(),
                    })?;
            if command.trim().is_empty() {
                return Err(ValidationError::EmptyCommand {
                    step: step.name.clone(),
                });
            }
            if step.has_facet_refs() {
                return Err(disallow(
                    "policy/knowledge/instruction/output_contract/input_contracts",
                ));
            }
            if step.inline_prompt.is_some() {
                return Err(disallow("inline_prompt"));
            }
            if step.collect.is_some() {
                return Err(disallow("collect"));
            }
            if step.parallel_children.is_some() {
                return Err(disallow("parallel_children"));
            }
            if step.aggregate.is_some() {
                return Err(disallow("aggregate"));
            }
        }
        NodeType::Agent | NodeType::Approval => {
            if step.command.is_some() {
                return Err(disallow("command"));
            }
            if step.parallel_children.is_some() {
                return Err(disallow("parallel_children"));
            }
            // aggregate は呼び出し側の AggregateWithoutParallel 経路で検証されるため
            // ここでは除外する。
        }
        NodeType::Parallel => {
            if step.command.is_some() {
                return Err(disallow("command"));
            }
            if step.has_facet_refs() {
                return Err(disallow(
                    "policy/knowledge/instruction/output_contract/input_contracts",
                ));
            }
            if step.inline_prompt.is_some() {
                return Err(disallow("inline_prompt"));
            }
            if step.collect.is_some() {
                return Err(disallow("collect"));
            }
        }
    }
    Ok(())
}

fn validate_approval_rules(
    step_name: &str,
    rules: &[super::schema::TransitionRule],
) -> Result<(), ValidationError> {
    let reject_count = rules.iter().filter(|r| r.r#match == "reject").count();
    if rules.iter().any(|r| r.r#match != "reject") {
        return Err(ValidationError::InvalidApprovalRules {
            step: step_name.to_string(),
            reason: "match: reject 以外のruleは定義できません".to_string(),
        });
    }
    if reject_count > 1 {
        return Err(ValidationError::InvalidApprovalRules {
            step: step_name.to_string(),
            reason: "match: reject ruleは最大1件です".to_string(),
        });
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
            if PermissionMode::parse(v).is_err() {
                return Err(ValidationError::InvalidPermissionMode {
                    step: step_name.to_string(),
                    value: v.to_string(),
                });
            }
            Ok(())
        }
    }
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
        if let Some(ref model) = step.model {
            validate_model_format(&step.name, model)?;
            validate_model_registered(&step.name, model, &mut resolve_model)?;
        }
        if let Some(ref children) = step.parallel_children {
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
        if let Some(ref children) = step.parallel_children {
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

    let mut preceding_step_names: HashSet<&str> = HashSet::new();

    for step in &workflow.nodes {
        if let Err(e) = validate_node_type_fields(step) {
            errors.push(e);
        }
        if step.is_parallel() {
            // 新 schema では node_type=Parallel が型レベルで mode を排除する。
            let Some(children) = step.parallel_children.as_ref() else {
                errors.push(ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "parallelブロックには parallel_children が必要です".to_string(),
                });
                continue;
            };
            if children.is_empty() {
                errors.push(ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "parallelブロックには1つ以上の子ステップが必要です".to_string(),
                });
            }
            let child_names: HashSet<&str> = children.iter().map(|c| c.name.as_str()).collect();

            for child in children {
                if child.node_type != NodeType::Agent {
                    errors.push(ValidationError::ParallelChildNotAuto {
                        parent: step.name.clone(),
                        child: child.name.clone(),
                    });
                }
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
                if let Some(ref refs) = child.pass_output_from {
                    for r in refs {
                        if child_names.contains(r.as_str()) {
                            errors.push(ValidationError::ParallelChildSiblingRef {
                                parent: step.name.clone(),
                                child: child.name.clone(),
                                reference: r.clone(),
                            });
                        }
                        if !referenceable_step_names.contains(r.as_str()) {
                            errors.push(ValidationError::UnknownOutputFrom {
                                step: child.name.clone(),
                                reference: r.clone(),
                            });
                        }
                    }
                }
            }

            if let Some(ref agg) = step.aggregate {
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
            // 新 schema では node_type が型レベルで必須・列挙のため、旧 schema の
            // MissingMode / InteractiveModeNotAllowed 検査は YAML deserialize 段階で吸収される。
            if step.node_type == NodeType::Approval {
                if let Err(e) = validate_approval_rules(&step.name, &step.transition_rules) {
                    errors.push(e);
                }
            }
            if let Err(e) = validate_required_permission(&step.name, step.permission.as_deref()) {
                errors.push(e);
            }
            if step.aggregate.is_some() {
                errors.push(ValidationError::AggregateWithoutParallel {
                    step: step.name.clone(),
                });
            }
            if let Some(err) = check_missing_facet(step) {
                errors.push(err);
            }
            for rule in &step.transition_rules {
                if !transition_target_names.contains(rule.next.as_str()) {
                    errors.push(ValidationError::UnknownNextStep {
                        step: step.name.clone(),
                        next: rule.next.clone(),
                    });
                }
            }
            if let Some(ref refs) = step.pass_output_from {
                for r in refs {
                    if !referenceable_step_names.contains(r.as_str()) {
                        errors.push(ValidationError::UnknownOutputFrom {
                            step: step.name.clone(),
                            reference: r.clone(),
                        });
                    }
                }
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

        // on_exhausted の参照先検証
        if let Some(ref guard) = step.cycle_guard {
            if let Some(ref target) = guard.on_exhausted {
                if !transition_target_names.contains(target.as_str()) {
                    errors.push(ValidationError::UnknownOnExhausted {
                        step: step.name.clone(),
                        target: target.clone(),
                    });
                }
            }
        }

        // resets_cycle_for の参照先検証
        if let Some(ref targets) = step.resets_cycle_for {
            for target in targets {
                if !transition_target_names.contains(target.as_str()) {
                    errors.push(ValidationError::UnknownResetsCycleFor {
                        step: step.name.clone(),
                        target: target.clone(),
                    });
                }
                let target_step = workflow.nodes.iter().find(|s| s.name == *target);
                if let Some(ts) = target_step {
                    if ts.cycle_guard.is_none() {
                        errors.push(ValidationError::ResetsCycleForNonGuardedStep {
                            step: step.name.clone(),
                            target: target.clone(),
                        });
                    }
                }
            }
        }

        preceding_step_names.insert(&step.name);
        if let Some(ref children) = step.parallel_children {
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }

    // on_exhausted の循環参照検出
    for step in &workflow.nodes {
        if let Some(ref guard) = step.cycle_guard {
            if let Some(ref target) = guard.on_exhausted {
                let mut visited = vec![step.name.clone()];
                let mut current = target.clone();
                loop {
                    if visited.contains(&current) {
                        visited.push(current);
                        errors.push(ValidationError::CircularOnExhausted { cycle: visited });
                        break;
                    }
                    visited.push(current.clone());
                    let next = workflow
                        .nodes
                        .iter()
                        .find(|s| s.name == current)
                        .and_then(|s| s.cycle_guard.as_ref())
                        .and_then(|g| g.on_exhausted.as_ref());
                    match next {
                        Some(n) => current = n.clone(),
                        None => break,
                    }
                }
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::{
        ChildNodeDefinition, CollectConfig, CycleGuard, NodeDefinition, ParallelAggregate,
        ReduceStrategy, TransitionRule,
    };

    fn make_workflow(nodes: Vec<NodeDefinition>) -> Workflow {
        Workflow {
            name: "test".to_string(),
            description: "test workflow".to_string(),
            builtin: false,
            nodes,
        }
    }

    fn resolve_from_set(valid: &HashSet<String>, model: &str) -> Result<Option<String>, String> {
        Ok(valid.contains(model).then(|| "backend".to_string()))
    }

    fn make_step(name: &str, node_type: NodeType, rules: Vec<TransitionRule>) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type,
            instruction: Some("implement".to_string()),
            transition_rules: rules,
            permission: Some("edit".to_string()),
            ..NodeDefinition::default()
        }
    }

    fn make_parallel_step(name: &str) -> ChildNodeDefinition {
        ChildNodeDefinition {
            name: name.to_string(),
            node_type: NodeType::Agent,
            instruction: Some("review".to_string()),
            permission: Some("edit".to_string()),
            ..ChildNodeDefinition::default()
        }
    }

    fn make_parallel_block(
        name: &str,
        children: Vec<ChildNodeDefinition>,
        aggregate: Option<ParallelAggregate>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type: NodeType::Parallel,
            parallel_children: Some(children),
            aggregate,
            permission: Some("edit".to_string()),
            ..NodeDefinition::default()
        }
    }

    // ---- 既存テスト ----

    #[test]
    fn valid_workflow_passes() {
        let wf = make_workflow(vec![
            make_step("plan", NodeType::Approval, vec![]),
            NodeDefinition {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 3,
                    on_exhausted: None,
                }),
                transition_rules: vec![TransitionRule {
                    r#match: "DONE".to_string(),
                    next: "plan".to_string(),
                }],
                ..make_step("implement", NodeType::Agent, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    // [02]: Interactive 概念が廃止された（StepMode 削除 + NodeType に Interactive 無し）ため、
    // 旧テスト `interactive_mode_fails_validation` は削除した。

    #[test]
    fn approval_step_allows_single_reject_rule() {
        let wf = make_workflow(vec![
            make_step("fix", NodeType::Agent, vec![]),
            make_step(
                "approval",
                NodeType::Approval,
                vec![TransitionRule {
                    r#match: "reject".to_string(),
                    next: "fix".to_string(),
                }],
            ),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn approval_step_rejects_non_reject_rule() {
        let wf = make_workflow(vec![
            make_step("fix", NodeType::Agent, vec![]),
            make_step(
                "approval",
                NodeType::Approval,
                vec![TransitionRule {
                    r#match: "NEEDS_FIX".to_string(),
                    next: "fix".to_string(),
                }],
            ),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidApprovalRules { ref step, .. } if step == "approval"
        ));
    }

    #[test]
    fn approval_step_rejects_multiple_reject_rules() {
        let wf = make_workflow(vec![
            make_step("fix", NodeType::Agent, vec![]),
            make_step(
                "approval",
                NodeType::Approval,
                vec![
                    TransitionRule {
                        r#match: "reject".to_string(),
                        next: "fix".to_string(),
                    },
                    TransitionRule {
                        r#match: "reject".to_string(),
                        next: "fix".to_string(),
                    },
                ],
            ),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InvalidApprovalRules { ref step, .. } if step == "approval"
        ));
    }

    #[test]
    fn invalid_transition_target_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            transition_rules: vec![TransitionRule {
                r#match: "DONE".to_string(),
                next: "nonexistent".to_string(),
            }],
            ..make_step("plan", NodeType::Agent, vec![])
        }]);
        let result = validate(&wf);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownNextStep { ref next, .. } if next == "nonexistent"
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
            make_step("plan", NodeType::Approval, vec![]),
            make_step("plan", NodeType::Agent, vec![]),
        ]);
        let result = validate(&wf);
        assert!(matches!(
            result.unwrap_err(),
            ValidationError::DuplicateStep { ref name } if name == "plan"
        ));
    }

    #[test]
    fn missing_facet_without_collect_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            instruction: None,
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::MissingFacet { ref step } if step == "step1"
        ));
    }

    #[test]
    fn facet_only_step_passes() {
        let wf = make_workflow(vec![NodeDefinition {
            policy: Some("coding".to_string()),
            instruction: Some("implement".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn collect_step_without_facets_passes() {
        let wf = make_workflow(vec![
            make_step("review_a", NodeType::Agent, vec![]),
            NodeDefinition {
                instruction: None,
                collect: Some(CollectConfig {
                    from: vec!["review_a".to_string()],
                    reduce: ReduceStrategy::Concat,
                }),
                ..make_step("collect_reviews", NodeType::Agent, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn unknown_output_from_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            pass_output_from: Some(vec!["nonexistent".to_string()]),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownOutputFrom { ref reference, .. } if reference == "nonexistent"
        ));
    }

    #[test]
    fn unknown_collect_from_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            instruction: None,
            collect: Some(CollectConfig {
                from: vec!["nonexistent".to_string()],
                reduce: ReduceStrategy::Concat,
            }),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownCollectFrom { ref reference, .. } if reference == "nonexistent"
        ));
    }

    #[test]
    fn valid_pass_output_from_passes() {
        let wf = make_workflow(vec![
            make_step("step_a", NodeType::Agent, vec![]),
            NodeDefinition {
                pass_output_from: Some(vec!["step_a".to_string()]),
                ..make_step("step_b", NodeType::Agent, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- 並列ブロック固有テスト ----

    #[test]
    fn valid_parallel_block_passes() {
        let wf = make_workflow(vec![
            make_step("implement", NodeType::Agent, vec![]),
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
            make_step("report", NodeType::Agent, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_block_without_aggregate_passes() {
        let wf = make_workflow(vec![
            make_step("implement", NodeType::Agent, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                None,
            ),
            make_step("report", NodeType::Agent, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // 新 schema では node_type=Parallel が型レベルで mode を排除するため、
    // 旧テスト `parallel_block_with_mode_fails` は廃止された（[02] 範囲）。

    #[test]
    fn parallel_child_not_auto_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![ChildNodeDefinition {
                node_type: NodeType::Approval,
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ParallelChildNotAuto { ref parent, ref child }
                if parent == "par" && child == "child1"
        ));
    }

    #[test]
    fn parallel_child_name_conflict_fails() {
        let wf = make_workflow(vec![
            make_step("conflict", NodeType::Agent, vec![]),
            make_parallel_block("par", vec![make_parallel_step("conflict")], None),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ParallelChildNameConflict { ref child } if child == "conflict"
        ));
    }

    #[test]
    fn aggregate_without_parallel_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            aggregate: Some(ParallelAggregate {
                all_match: Some("LGTM".to_string()),
                any_match: None,
                then: "implement".to_string(),
                r#else: "implement".to_string(),
            }),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::AggregateWithoutParallel { ref step } if step == "step1"
        ));
    }

    #[test]
    fn aggregate_both_match_fails() {
        let wf = make_workflow(vec![
            make_step("target", NodeType::Agent, vec![]),
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
            make_step("target", NodeType::Agent, vec![]),
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
    fn parallel_child_sibling_ref_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![
                make_parallel_step("child1"),
                ChildNodeDefinition {
                    pass_output_from: Some(vec!["child1".to_string()]),
                    ..make_parallel_step("child2")
                },
            ],
            None,
        )]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ParallelChildSiblingRef { ref parent, ref child, ref reference }
                if parent == "par" && child == "child2" && reference == "child1"
        ));
    }

    #[test]
    fn parallel_child_missing_facet_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![ChildNodeDefinition {
                instruction: None,
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
    fn parallel_child_pass_output_from_valid_global_step() {
        let wf = make_workflow(vec![
            make_step("plan", NodeType::Agent, vec![]),
            make_parallel_block(
                "par",
                vec![ChildNodeDefinition {
                    pass_output_from: Some(vec!["plan".to_string()]),
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
                instruction: None,
                collect: Some(CollectConfig {
                    from: vec!["arch-review".to_string(), "security-review".to_string()],
                    reduce: ReduceStrategy::Concat,
                }),
                ..make_step("report", NodeType::Agent, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn empty_parallel_children_fails() {
        let wf = make_workflow(vec![
            make_step("implement", NodeType::Agent, vec![]),
            make_parallel_block("parallel-review", vec![], None),
            make_step("report", NodeType::Agent, vec![]),
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
            make_step("implement", NodeType::Agent, vec![]),
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
            make_step("report", NodeType::Agent, vec![]),
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
            make_step("implement", NodeType::Agent, vec![]),
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
            make_step("report", NodeType::Agent, vec![]),
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
            make_step("implement", NodeType::Agent, vec![]),
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
            make_step("report", NodeType::Agent, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_pass_output_from_subsequent_step_passes() {
        // parallel block より後に定義されたステップへの後方参照は許可される
        // （出力が未生成の場合は空として扱われる）
        let wf = make_workflow(vec![
            make_step("plan", NodeType::Agent, vec![]),
            make_parallel_block(
                "par",
                vec![ChildNodeDefinition {
                    pass_output_from: Some(vec!["report".to_string()]),
                    ..make_parallel_step("child1")
                }],
                None,
            ),
            make_step("report", NodeType::Agent, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_pass_output_from_preceding_step_passes() {
        // parallel block より前に定義されたステップへの参照はOK
        let wf = make_workflow(vec![
            make_step("plan", NodeType::Agent, vec![]),
            make_step("implement", NodeType::Agent, vec![]),
            make_parallel_block(
                "par",
                vec![ChildNodeDefinition {
                    pass_output_from: Some(vec!["plan".to_string(), "implement".to_string()]),
                    ..make_parallel_step("child1")
                }],
                None,
            ),
            make_step("report", NodeType::Agent, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_pass_output_from_prior_parallel_child_passes() {
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
                vec![ChildNodeDefinition {
                    pass_output_from: Some(vec!["review-a".to_string()]),
                    ..make_parallel_step("summarize")
                }],
                None,
            ),
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- on_exhausted バリデーション ----

    #[test]
    fn on_exhausted_valid_target_passes() {
        let wf = make_workflow(vec![
            NodeDefinition {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 2,
                    on_exhausted: Some("approval".to_string()),
                }),
                transition_rules: vec![TransitionRule {
                    r#match: ".*".to_string(),
                    next: "approval".to_string(),
                }],
                ..make_step("fix", NodeType::Agent, vec![])
            },
            make_step("approval", NodeType::Approval, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn on_exhausted_unknown_target_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            cycle_guard: Some(CycleGuard {
                max_iterations: 2,
                on_exhausted: Some("nonexistent".to_string()),
            }),
            ..make_step("fix", NodeType::Agent, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownOnExhausted { ref step, ref target }
                if step == "fix" && target == "nonexistent"
        ));
    }

    #[test]
    fn on_exhausted_circular_fails() {
        let wf = make_workflow(vec![
            NodeDefinition {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 2,
                    on_exhausted: Some("step_b".to_string()),
                }),
                ..make_step("step_a", NodeType::Agent, vec![])
            },
            NodeDefinition {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 2,
                    on_exhausted: Some("step_a".to_string()),
                }),
                ..make_step("step_b", NodeType::Agent, vec![])
            },
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(err, ValidationError::CircularOnExhausted { .. }));
    }

    // ---- resets_cycle_for バリデーション ----

    #[test]
    fn resets_cycle_for_valid_target_passes() {
        let wf = make_workflow(vec![
            NodeDefinition {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 3,
                    on_exhausted: None,
                }),
                ..make_step("fix", NodeType::Agent, vec![])
            },
            NodeDefinition {
                resets_cycle_for: Some(vec!["fix".to_string()]),
                ..make_step("approval", NodeType::Approval, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn resets_cycle_for_unknown_target_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            resets_cycle_for: Some(vec!["nonexistent".to_string()]),
            ..make_step("approval", NodeType::Approval, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownResetsCycleFor { ref step, ref target }
                if step == "approval" && target == "nonexistent"
        ));
    }

    #[test]
    fn resets_cycle_for_non_guarded_step_fails() {
        let wf = make_workflow(vec![
            make_step("fix", NodeType::Agent, vec![]),
            NodeDefinition {
                resets_cycle_for: Some(vec!["fix".to_string()]),
                ..make_step("approval", NodeType::Approval, vec![])
            },
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::ResetsCycleForNonGuardedStep { ref step, ref target }
                if step == "approval" && target == "fix"
        ));
    }

    // ---- pass_output_from 後方参照 ----

    #[test]
    fn pass_output_from_backward_reference_passes() {
        // 定義順で後方のステップを pass_output_from で参照できる
        let wf = make_workflow(vec![
            NodeDefinition {
                pass_output_from: Some(vec!["step_b".to_string()]),
                ..make_step("step_a", NodeType::Agent, vec![])
            },
            make_step("step_b", NodeType::Agent, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn inline_prompt_step_without_facets_passes() {
        let wf = make_workflow(vec![NodeDefinition {
            instruction: None,
            inline_prompt: Some("Do analysis".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(validate(&wf).is_ok());
    }

    // ---- permission バリデーション ----

    #[test]
    fn valid_permission_readonly_passes() {
        let wf = make_workflow(vec![NodeDefinition {
            permission: Some("readonly".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn valid_permission_edit_passes() {
        let wf = make_workflow(vec![NodeDefinition {
            permission: Some("edit".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn valid_permission_full_passes() {
        let wf = make_workflow(vec![NodeDefinition {
            permission: Some("full".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn legacy_permission_accept_edits_rejected() {
        for legacy in ["acceptEdits", "bypassPermissions", "plan", "default"] {
            let wf = make_workflow(vec![NodeDefinition {
                permission: Some(legacy.to_string()),
                ..make_step("step1", NodeType::Agent, vec![])
            }]);
            let err = validate(&wf).unwrap_err();
            assert!(matches!(
                err,
                ValidationError::InvalidPermissionMode { ref step, ref value }
                    if step == "step1" && value == legacy
            ));
            assert!(
                err.to_string().contains("readonly, edit, full"),
                "error must include allowed list, got: {err}"
            );
        }
    }

    #[test]
    fn invalid_permission_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            permission: Some("invalid-mode".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref step, ref value }
                if step == "step1" && value == "invalid-mode"
        ));
        assert!(err.to_string().contains("readonly, edit, full"));
    }

    #[test]
    fn empty_permission_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            permission: Some(String::new()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::InvalidPermissionMode { ref step, ref value }
                if step == "step1" && value.is_empty()
        ));
        assert!(err.to_string().contains("readonly, edit, full"));
    }

    #[test]
    fn invalid_permission_on_parallel_child_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![ChildNodeDefinition {
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
            vec![ChildNodeDefinition {
                permission: Some("full".to_string()),
                ..make_parallel_step("child1")
            }],
            None,
        )]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn step_without_permission_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            permission: None,
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::MissingPermissionMode { ref step } if step == "step1"
        ));
        assert!(err.to_string().contains("readonly, edit, full"));
    }

    #[test]
    fn parallel_child_without_permission_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![ChildNodeDefinition {
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
        let wf = make_workflow(vec![NodeDefinition {
            permission: None,
            ..make_parallel_block(
                "par",
                vec![ChildNodeDefinition {
                    permission: Some("edit".to_string()),
                    ..make_parallel_step("child1")
                }],
                None,
            )
        }]);
        assert!(validate(&wf).is_ok());
    }

    // ---- model バリデーション (validate_models) ----

    #[test]
    fn validate_models_valid_model_passes() {
        let wf = make_workflow(vec![NodeDefinition {
            model: Some("haiku".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let valid = HashSet::from(["haiku".to_string(), "opus-4".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_unknown_model_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            model: Some("unknown-model".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
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
            vec![ChildNodeDefinition {
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
        let wf = make_workflow(vec![NodeDefinition {
            model: Some("shared".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
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
        let wf = make_workflow(vec![make_step("step1", NodeType::Agent, vec![])]);
        let valid = HashSet::from(["haiku".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_ok());
    }

    #[test]
    fn validate_models_valid_model_on_parallel_child_passes() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![ChildNodeDefinition {
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
        let wf = make_workflow(vec![NodeDefinition {
            model: Some(String::new()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
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
        let wf = make_workflow(vec![NodeDefinition {
            model: Some("   ".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let valid = HashSet::from(["   ".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_err());
    }

    #[test]
    fn validate_models_rejects_control_character_model() {
        let wf = make_workflow(vec![NodeDefinition {
            model: Some("a\u{0001}b".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let valid = HashSet::from(["a\u{0001}b".to_string()]);
        assert!(validate_models(&wf, |model| resolve_from_set(&valid, model)).is_err());
    }

    // ---- [02] bash 種別の専用 validation ----

    #[test]
    fn bash_node_with_command_passes_when_facets_absent() {
        let wf = make_workflow(vec![NodeDefinition {
            node_type: NodeType::Bash,
            command: Some("cargo build".to_string()),
            instruction: None,
            permission: Some("edit".to_string()),
            ..NodeDefinition::default()
        }
        .into_named("build")]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn bash_node_without_command_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            name: "build".to_string(),
            node_type: NodeType::Bash,
            command: None,
            permission: Some("edit".to_string()),
            ..NodeDefinition::default()
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::MissingCommand { ref step } if step == "build"
        ));
    }

    #[test]
    fn bash_node_with_empty_command_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            name: "build".to_string(),
            node_type: NodeType::Bash,
            command: Some("   ".to_string()),
            permission: Some("edit".to_string()),
            ..NodeDefinition::default()
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::EmptyCommand { ref step } if step == "build"
        ));
    }

    #[test]
    fn bash_node_with_facet_refs_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            name: "build".to_string(),
            node_type: NodeType::Bash,
            command: Some("cargo build".to_string()),
            policy: Some("coding".to_string()),
            permission: Some("edit".to_string()),
            ..NodeDefinition::default()
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::DisallowedFieldForNodeType { ref step, .. } if step == "build"
        ));
    }

    #[test]
    fn bash_node_with_inline_prompt_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            name: "build".to_string(),
            node_type: NodeType::Bash,
            command: Some("cargo build".to_string()),
            inline_prompt: Some("hello".to_string()),
            permission: Some("edit".to_string()),
            ..NodeDefinition::default()
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::DisallowedFieldForNodeType { ref step, .. } if step == "build"
        ));
    }

    #[test]
    fn agent_node_with_command_fails() {
        let wf = make_workflow(vec![NodeDefinition {
            command: Some("cargo build".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::DisallowedFieldForNodeType { ref step, field, .. }
                if step == "step1" && field == "command"
        ));
    }

    #[test]
    fn parallel_node_with_command_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![make_parallel_step("child1")],
            Some(ParallelAggregate {
                all_match: Some("LGTM".to_string()),
                any_match: None,
                then: "par".to_string(),
                r#else: "par".to_string(),
            }),
        )]);
        let mut wf_mut = wf;
        wf_mut.nodes[0].command = Some("echo".to_string());
        let err = validate(&wf_mut).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::DisallowedFieldForNodeType { field, .. } if field == "command"
        ));
    }

    #[test]
    fn parallel_node_with_facet_refs_on_block_fails() {
        let mut wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![make_parallel_step("child1")],
            Some(ParallelAggregate {
                all_match: Some("LGTM".to_string()),
                any_match: None,
                then: "par".to_string(),
                r#else: "par".to_string(),
            }),
        )]);
        wf.nodes[0].policy = Some("coding".to_string());
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::DisallowedFieldForNodeType { .. }
        ));
    }

    // ---- DoS ガードのテスト ----

    #[test]
    fn too_many_nodes_fails() {
        let nodes: Vec<NodeDefinition> = (0..MAX_NODES_PER_WORKFLOW + 1)
            .map(|i| make_step(&format!("step{i}"), NodeType::Agent, vec![]))
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
        let children: Vec<ChildNodeDefinition> = (0..MAX_NODES_PER_WORKFLOW)
            .take(MAX_PARALLEL_CHILDREN)
            .map(|i| make_parallel_step(&format!("c{i}")))
            .collect();
        // children は MAX_PARALLEL_CHILDREN にクリップされる。
        // ガードを越えるため、複数 parallel block で children を積み増す。
        let need_blocks = (MAX_NODES_PER_WORKFLOW + 1).div_ceil(MAX_PARALLEL_CHILDREN) + 1;
        let mut nodes: Vec<NodeDefinition> = Vec::new();
        for b in 0..need_blocks {
            let block_children: Vec<ChildNodeDefinition> = children
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
        let children: Vec<ChildNodeDefinition> = (0..MAX_PARALLEL_CHILDREN + 1)
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

    trait IntoNamed {
        fn into_named(self, name: &str) -> Self;
    }

    impl IntoNamed for super::super::schema::NodeDefinition {
        fn into_named(mut self, name: &str) -> Self {
            self.name = name.to_string();
            self
        }
    }

    // ---- validate_facet_refs ----

    #[test]
    fn validate_facet_refs_passes_when_all_contracts_exist() {
        let wf = make_workflow(vec![NodeDefinition {
            output_contract: Some("output-contract".to_string()),
            input_contracts: Some(vec![
                "input-contract-a".to_string(),
                "input-contract-b".to_string(),
            ]),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let known: HashSet<&str> =
            HashSet::from(["output-contract", "input-contract-a", "input-contract-b"]);
        assert!(validate_facet_refs(&wf, |k| known.contains(k)).is_ok());
    }

    #[test]
    fn validate_facet_refs_detects_missing_output_contract() {
        let wf = make_workflow(vec![NodeDefinition {
            output_contract: Some("nonexistent".to_string()),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let err = validate_facet_refs(&wf, |_| false).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownContractRef { ref step, slot, ref key }
                if step == "step1" && slot == "output_contract" && key == "nonexistent"
        ));
    }

    #[test]
    fn validate_facet_refs_detects_missing_input_contract() {
        let wf = make_workflow(vec![NodeDefinition {
            input_contracts: Some(vec!["unknown-key".to_string()]),
            ..make_step("step1", NodeType::Agent, vec![])
        }]);
        let err = validate_facet_refs(&wf, |_| false).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownContractRef { ref step, slot, ref key }
                if step == "step1" && slot == "input_contracts" && key == "unknown-key"
        ));
    }

    #[test]
    fn validate_facet_refs_inspects_parallel_children() {
        let child = ChildNodeDefinition {
            input_contracts: Some(vec!["nope".to_string()]),
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
        let err = validate_facet_refs(&wf, |_| false).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownContractRef { ref step, slot, ref key }
                if step == "child1" && slot == "input_contracts" && key == "nope"
        ));
    }
}
