use super::schema::{StepMode, Workflow};
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
    /// 通常step（parallel なし）で mode が未指定
    MissingMode {
        step: String,
    },
    /// parallel block で mode が指定されている
    ParallelBlockHasMode {
        step: String,
    },
    /// 並列子stepが auto 以外の mode
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
    /// interactive mode は廃止済み
    InteractiveModeNotAllowed {
        step: String,
    },
    /// approval step の rules は最大1件の match: reject のみ許可
    InvalidApprovalRules {
        step: String,
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
            Self::MissingMode { step } => {
                write!(f, "ステップ '{step}' にはmodeが必要です（parallelブロックを除く）")
            }
            Self::ParallelBlockHasMode { step } => {
                write!(f, "parallelブロック '{step}' にmodeは指定できません")
            }
            Self::ParallelChildNotAuto { parent, child } => {
                write!(
                    f,
                    "parallelブロック '{parent}' の子ステップ '{child}' はautoモードのみ許可されています"
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
            Self::InteractiveModeNotAllowed { step } => write!(
                f,
                "ステップ '{step}' のmode: interactiveは廃止されています。対話を伴うstepはmode: approvalを使用してください"
            ),
            Self::InvalidApprovalRules { step, reason } => {
                write!(f, "approvalステップ '{step}' のrulesが不正です: {reason}")
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

pub fn validate(workflow: &Workflow) -> Result<(), ValidationError> {
    validate_name(&workflow.name)?;

    if workflow.steps.is_empty() {
        return Err(ValidationError::EmptySteps);
    }

    // 遷移先名前空間: トップレベルstep名のみ（aggregate.then/else, rule.nextの検証用）
    let mut transition_target_names = HashSet::new();
    // 参照可能名前空間: トップレベルstep名 + 並列子step名（pass_output_from等の検証用）
    let mut referenceable_step_names = HashSet::new();
    for step in &workflow.steps {
        if !transition_target_names.insert(step.name.as_str()) {
            return Err(ValidationError::DuplicateStep {
                name: step.name.clone(),
            });
        }
        referenceable_step_names.insert(step.name.as_str());
        // 並列子step名は参照可能名前空間にのみ追加（遷移先には不可）
        if let Some(ref children) = step.parallel {
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

    for step in &workflow.steps {
        if step.is_parallel_block() {
            // --- parallel block 固有のバリデーション ---

            // parallel block に mode が指定されていたらエラー
            if step.mode.is_some() {
                return Err(ValidationError::ParallelBlockHasMode {
                    step: step.name.clone(),
                });
            }

            let children = step.parallel.as_ref().unwrap();
            if children.is_empty() {
                return Err(ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "parallelブロックには1つ以上の子ステップが必要です".to_string(),
                });
            }
            let child_names: HashSet<&str> = children.iter().map(|c| c.name.as_str()).collect();

            for child in children {
                // 子step は auto のみ
                if child.mode != StepMode::Auto {
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

            // 通常step は mode 必須
            if step.mode.is_none() {
                return Err(ValidationError::MissingMode {
                    step: step.name.clone(),
                });
            }
            if step.mode == Some(StepMode::Interactive) {
                return Err(ValidationError::InteractiveModeNotAllowed {
                    step: step.name.clone(),
                });
            }
            if step.mode == Some(StepMode::Approval) {
                validate_approval_rules(&step.name, &step.rules)?;
            }

            // aggregate が parallel なしで指定されている場合はエラー
            if step.aggregate.is_some() {
                return Err(ValidationError::AggregateWithoutParallel {
                    step: step.name.clone(),
                });
            }

            // collect なしの step: ファセット参照または inline_prompt が必要
            if step.collect.is_none() && !step.has_facet_refs() && step.inline_prompt.is_none() {
                return Err(ValidationError::MissingFacet {
                    step: step.name.clone(),
                });
            }

            // rules の遷移先チェック（トップレベルstepのみ）
            for rule in &step.rules {
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
                        let referenced_step = workflow.steps.iter().find(|s| s.name == *r);
                        if let Some(rs) = referenced_step {
                            if rs.rules.is_empty() && !rs.is_parallel_block() {
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
                let target_step = workflow.steps.iter().find(|s| s.name == *target);
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
        if let Some(ref children) = step.parallel {
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }

    // on_exhausted の循環参照検出
    for step in &workflow.steps {
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
                        .steps
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

    if workflow.steps.is_empty() {
        errors.push(ValidationError::EmptySteps);
        return errors;
    }

    // 名前空間構築: 重複があれば蓄積するが、以降のチェックは続行
    let mut transition_target_names = HashSet::new();
    let mut referenceable_step_names = HashSet::new();
    let mut has_dup = false;
    for step in &workflow.steps {
        if !transition_target_names.insert(step.name.as_str()) {
            errors.push(ValidationError::DuplicateStep {
                name: step.name.clone(),
            });
            has_dup = true;
        }
        referenceable_step_names.insert(step.name.as_str());
        if let Some(ref children) = step.parallel {
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

    for step in &workflow.steps {
        if step.is_parallel_block() {
            if step.mode.is_some() {
                errors.push(ValidationError::ParallelBlockHasMode {
                    step: step.name.clone(),
                });
            }

            let children = step.parallel.as_ref().unwrap();
            if children.is_empty() {
                errors.push(ValidationError::AggregateInvalidConfig {
                    step: step.name.clone(),
                    reason: "parallelブロックには1つ以上の子ステップが必要です".to_string(),
                });
            }
            let child_names: HashSet<&str> = children.iter().map(|c| c.name.as_str()).collect();

            for child in children {
                if child.mode != StepMode::Auto {
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
            if step.mode.is_none() {
                errors.push(ValidationError::MissingMode {
                    step: step.name.clone(),
                });
            }
            if step.mode == Some(StepMode::Interactive) {
                errors.push(ValidationError::InteractiveModeNotAllowed {
                    step: step.name.clone(),
                });
            }
            if step.mode == Some(StepMode::Approval) {
                if let Err(e) = validate_approval_rules(&step.name, &step.rules) {
                    errors.push(e);
                }
            }
            if step.aggregate.is_some() {
                errors.push(ValidationError::AggregateWithoutParallel {
                    step: step.name.clone(),
                });
            }
            if step.collect.is_none() && !step.has_facet_refs() && step.inline_prompt.is_none() {
                errors.push(ValidationError::MissingFacet {
                    step: step.name.clone(),
                });
            }
            for rule in &step.rules {
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
                let target_step = workflow.steps.iter().find(|s| s.name == *target);
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
        if let Some(ref children) = step.parallel {
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }

    // on_exhausted の循環参照検出
    for step in &workflow.steps {
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
                        .steps
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
        AggregateConfig, CollectConfig, CycleGuard, ParallelStep, ReduceStrategy, Step, StepMode,
        TransitionRule,
    };

    fn make_workflow(steps: Vec<Step>) -> Workflow {
        Workflow {
            name: "test".to_string(),
            description: "test workflow".to_string(),
            builtin: false,
            steps,
        }
    }

    fn make_step(name: &str, mode: StepMode, rules: Vec<TransitionRule>) -> Step {
        Step {
            name: name.to_string(),
            mode: Some(mode),
            persona: None,
            policy: None,
            knowledge: None,
            instruction: Some("implement".to_string()),
            output_contract: None,
            rules,
            cycle_guard: None,
            pass_previous_response: None,
            pass_output_from: None,
            inline_prompt: None,
            collect: None,
            parallel: None,
            aggregate: None,
            resets_cycle_for: None,
        }
    }

    fn make_parallel_step(name: &str) -> ParallelStep {
        ParallelStep {
            name: name.to_string(),
            mode: StepMode::Auto,
            persona: Some("reviewer".to_string()),
            policy: None,
            knowledge: None,
            instruction: Some("review".to_string()),
            output_contract: None,
            pass_previous_response: None,
            pass_output_from: None,
        }
    }

    fn make_parallel_block(
        name: &str,
        children: Vec<ParallelStep>,
        aggregate: Option<AggregateConfig>,
    ) -> Step {
        Step {
            name: name.to_string(),
            mode: None,
            persona: None,
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            rules: vec![],
            cycle_guard: None,
            pass_previous_response: None,
            pass_output_from: None,
            inline_prompt: None,
            collect: None,
            parallel: Some(children),
            aggregate,
            resets_cycle_for: None,
        }
    }

    // ---- 既存テスト ----

    #[test]
    fn valid_workflow_passes() {
        let wf = make_workflow(vec![
            make_step("plan", StepMode::Approval, vec![]),
            Step {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 3,
                    on_exhausted: None,
                }),
                rules: vec![TransitionRule {
                    r#match: "DONE".to_string(),
                    next: "plan".to_string(),
                }],
                ..make_step("implement", StepMode::Auto, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn interactive_mode_fails_validation() {
        let wf = make_workflow(vec![make_step("plan", StepMode::Interactive, vec![])]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::InteractiveModeNotAllowed { ref step } if step == "plan"
        ));
    }

    #[test]
    fn approval_step_allows_single_reject_rule() {
        let wf = make_workflow(vec![
            make_step("fix", StepMode::Auto, vec![]),
            make_step(
                "approval",
                StepMode::Approval,
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
            make_step("fix", StepMode::Auto, vec![]),
            make_step(
                "approval",
                StepMode::Approval,
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
            make_step("fix", StepMode::Auto, vec![]),
            make_step(
                "approval",
                StepMode::Approval,
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
        let wf = make_workflow(vec![Step {
            rules: vec![TransitionRule {
                r#match: "DONE".to_string(),
                next: "nonexistent".to_string(),
            }],
            ..make_step("plan", StepMode::Auto, vec![])
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
            make_step("plan", StepMode::Approval, vec![]),
            make_step("plan", StepMode::Auto, vec![]),
        ]);
        let result = validate(&wf);
        assert!(matches!(
            result.unwrap_err(),
            ValidationError::DuplicateStep { ref name } if name == "plan"
        ));
    }

    #[test]
    fn missing_facet_without_collect_fails() {
        let wf = make_workflow(vec![Step {
            instruction: None,
            ..make_step("step1", StepMode::Auto, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::MissingFacet { ref step } if step == "step1"
        ));
    }

    #[test]
    fn facet_only_step_passes() {
        let wf = make_workflow(vec![Step {
            persona: Some("coder".to_string()),
            instruction: Some("implement".to_string()),
            ..make_step("step1", StepMode::Auto, vec![])
        }]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn collect_step_without_facets_passes() {
        let wf = make_workflow(vec![
            make_step("review_a", StepMode::Auto, vec![]),
            Step {
                instruction: None,
                collect: Some(CollectConfig {
                    from: vec!["review_a".to_string()],
                    reduce: ReduceStrategy::Concat,
                }),
                ..make_step("collect_reviews", StepMode::Auto, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn unknown_output_from_fails() {
        let wf = make_workflow(vec![Step {
            pass_output_from: Some(vec!["nonexistent".to_string()]),
            ..make_step("step1", StepMode::Auto, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownOutputFrom { ref reference, .. } if reference == "nonexistent"
        ));
    }

    #[test]
    fn unknown_collect_from_fails() {
        let wf = make_workflow(vec![Step {
            instruction: None,
            collect: Some(CollectConfig {
                from: vec!["nonexistent".to_string()],
                reduce: ReduceStrategy::Concat,
            }),
            ..make_step("step1", StepMode::Auto, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::UnknownCollectFrom { ref reference, .. } if reference == "nonexistent"
        ));
    }

    #[test]
    fn valid_pass_output_from_passes() {
        let wf = make_workflow(vec![
            make_step("step_a", StepMode::Auto, vec![]),
            Step {
                pass_output_from: Some(vec!["step_a".to_string()]),
                ..make_step("step_b", StepMode::Auto, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    // ---- 並列ブロック固有テスト ----

    #[test]
    fn valid_parallel_block_passes() {
        let wf = make_workflow(vec![
            make_step("implement", StepMode::Auto, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(AggregateConfig {
                    all_match: Some("LGTM".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", StepMode::Auto, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_block_without_aggregate_passes() {
        let wf = make_workflow(vec![
            make_step("implement", StepMode::Auto, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                None,
            ),
            make_step("report", StepMode::Auto, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_block_with_mode_fails() {
        let wf = make_workflow(vec![Step {
            mode: Some(StepMode::Auto),
            parallel: Some(vec![make_parallel_step("child1")]),
            ..make_parallel_block("bad", vec![], None)
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ParallelBlockHasMode { ref step } if step == "bad"
        ));
    }

    #[test]
    fn parallel_child_not_auto_fails() {
        let wf = make_workflow(vec![make_parallel_block(
            "par",
            vec![ParallelStep {
                mode: StepMode::Approval,
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
            make_step("conflict", StepMode::Auto, vec![]),
            make_parallel_block("par", vec![make_parallel_step("conflict")], None),
        ]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::ParallelChildNameConflict { ref child } if child == "conflict"
        ));
    }

    #[test]
    fn aggregate_without_parallel_fails() {
        let wf = make_workflow(vec![Step {
            aggregate: Some(AggregateConfig {
                all_match: Some("LGTM".to_string()),
                any_match: None,
                then: "implement".to_string(),
                r#else: "implement".to_string(),
            }),
            ..make_step("step1", StepMode::Auto, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::AggregateWithoutParallel { ref step } if step == "step1"
        ));
    }

    #[test]
    fn aggregate_both_match_fails() {
        let wf = make_workflow(vec![
            make_step("target", StepMode::Auto, vec![]),
            make_parallel_block(
                "par",
                vec![make_parallel_step("child1")],
                Some(AggregateConfig {
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
            make_step("target", StepMode::Auto, vec![]),
            make_parallel_block(
                "par",
                vec![make_parallel_step("child1")],
                Some(AggregateConfig {
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
            Some(AggregateConfig {
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
                ParallelStep {
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
            vec![ParallelStep {
                persona: None,
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

    #[test]
    fn normal_step_missing_mode_fails() {
        let wf = make_workflow(vec![Step {
            mode: None,
            ..make_step("step1", StepMode::Auto, vec![])
        }]);
        assert!(matches!(
            validate(&wf).unwrap_err(),
            ValidationError::MissingMode { ref step } if step == "step1"
        ));
    }

    #[test]
    fn parallel_child_pass_output_from_valid_global_step() {
        let wf = make_workflow(vec![
            make_step("plan", StepMode::Auto, vec![]),
            make_parallel_block(
                "par",
                vec![ParallelStep {
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
                Some(AggregateConfig {
                    all_match: Some("LGTM".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "report".to_string(),
                }),
            ),
            Step {
                instruction: None,
                collect: Some(CollectConfig {
                    from: vec!["arch-review".to_string(), "security-review".to_string()],
                    reduce: ReduceStrategy::Concat,
                }),
                ..make_step("report", StepMode::Auto, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn empty_parallel_children_fails() {
        let wf = make_workflow(vec![
            make_step("implement", StepMode::Auto, vec![]),
            make_parallel_block("parallel-review", vec![], None),
            make_step("report", StepMode::Auto, vec![]),
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
            make_step("implement", StepMode::Auto, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(AggregateConfig {
                    all_match: Some("[invalid(regex".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", StepMode::Auto, vec![]),
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
            make_step("implement", StepMode::Auto, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(AggregateConfig {
                    all_match: None,
                    any_match: Some("(unclosed".to_string()),
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", StepMode::Auto, vec![]),
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
            make_step("implement", StepMode::Auto, vec![]),
            make_parallel_block(
                "parallel-review",
                vec![
                    make_parallel_step("arch-review"),
                    make_parallel_step("security-review"),
                ],
                Some(AggregateConfig {
                    all_match: Some(r"<decision>(LGTM|APPROVED)</decision>".to_string()),
                    any_match: None,
                    then: "report".to_string(),
                    r#else: "implement".to_string(),
                }),
            ),
            make_step("report", StepMode::Auto, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_pass_output_from_subsequent_step_passes() {
        // parallel block より後に定義されたステップへの後方参照は許可される
        // （出力が未生成の場合は空として扱われる）
        let wf = make_workflow(vec![
            make_step("plan", StepMode::Auto, vec![]),
            make_parallel_block(
                "par",
                vec![ParallelStep {
                    pass_output_from: Some(vec!["report".to_string()]),
                    ..make_parallel_step("child1")
                }],
                None,
            ),
            make_step("report", StepMode::Auto, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn parallel_child_pass_output_from_preceding_step_passes() {
        // parallel block より前に定義されたステップへの参照はOK
        let wf = make_workflow(vec![
            make_step("plan", StepMode::Auto, vec![]),
            make_step("implement", StepMode::Auto, vec![]),
            make_parallel_block(
                "par",
                vec![ParallelStep {
                    pass_output_from: Some(vec!["plan".to_string(), "implement".to_string()]),
                    ..make_parallel_step("child1")
                }],
                None,
            ),
            make_step("report", StepMode::Auto, vec![]),
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
                vec![ParallelStep {
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
            Step {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 2,
                    on_exhausted: Some("approval".to_string()),
                }),
                rules: vec![TransitionRule {
                    r#match: ".*".to_string(),
                    next: "approval".to_string(),
                }],
                ..make_step("fix", StepMode::Auto, vec![])
            },
            make_step("approval", StepMode::Approval, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn on_exhausted_unknown_target_fails() {
        let wf = make_workflow(vec![Step {
            cycle_guard: Some(CycleGuard {
                max_iterations: 2,
                on_exhausted: Some("nonexistent".to_string()),
            }),
            ..make_step("fix", StepMode::Auto, vec![])
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
            Step {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 2,
                    on_exhausted: Some("step_b".to_string()),
                }),
                ..make_step("step_a", StepMode::Auto, vec![])
            },
            Step {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 2,
                    on_exhausted: Some("step_a".to_string()),
                }),
                ..make_step("step_b", StepMode::Auto, vec![])
            },
        ]);
        let err = validate(&wf).unwrap_err();
        assert!(matches!(err, ValidationError::CircularOnExhausted { .. }));
    }

    // ---- resets_cycle_for バリデーション ----

    #[test]
    fn resets_cycle_for_valid_target_passes() {
        let wf = make_workflow(vec![
            Step {
                cycle_guard: Some(CycleGuard {
                    max_iterations: 3,
                    on_exhausted: None,
                }),
                ..make_step("fix", StepMode::Auto, vec![])
            },
            Step {
                resets_cycle_for: Some(vec!["fix".to_string()]),
                ..make_step("approval", StepMode::Approval, vec![])
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn resets_cycle_for_unknown_target_fails() {
        let wf = make_workflow(vec![Step {
            resets_cycle_for: Some(vec!["nonexistent".to_string()]),
            ..make_step("approval", StepMode::Approval, vec![])
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
            make_step("fix", StepMode::Auto, vec![]),
            Step {
                resets_cycle_for: Some(vec!["fix".to_string()]),
                ..make_step("approval", StepMode::Approval, vec![])
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
            Step {
                pass_output_from: Some(vec!["step_b".to_string()]),
                ..make_step("step_a", StepMode::Auto, vec![])
            },
            make_step("step_b", StepMode::Auto, vec![]),
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn inline_prompt_step_without_facets_passes() {
        let wf = make_workflow(vec![Step {
            instruction: None,
            inline_prompt: Some("Do analysis".to_string()),
            ..make_step("step1", StepMode::Auto, vec![])
        }]);
        assert!(validate(&wf).is_ok());
    }
}
