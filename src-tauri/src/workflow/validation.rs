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
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => write!(f, "ワークフロー名が空です"),
            Self::InvalidChars { name } => write!(
                f,
                "ワークフロー名 '{name}' に使用できない文字が含まれています（英数字・ハイフン・アンダースコアのみ許可）"
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
                    "ステップ '{step}' にはファセット参照が必要です（collectステップのみ省略可）"
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
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
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

    // グローバル名前空間: 通常step名 + 並列子step名
    let mut step_names = HashSet::new();
    for step in &workflow.steps {
        if !step_names.insert(step.name.as_str()) {
            return Err(ValidationError::DuplicateStep {
                name: step.name.clone(),
            });
        }
        // 並列子step名もグローバル名前空間に追加
        if let Some(ref children) = step.parallel {
            for child in children {
                if !step_names.insert(child.name.as_str()) {
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
                        // 親parallel blockより前に定義されたステップのみ参照可能
                        if !preceding_step_names.contains(r.as_str()) {
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

                // then/else の遷移先存在チェック
                if !step_names.contains(agg.then.as_str()) {
                    return Err(ValidationError::AggregateUnknownTarget {
                        step: step.name.clone(),
                        target: agg.then.clone(),
                    });
                }
                if !step_names.contains(agg.r#else.as_str()) {
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

            // aggregate が parallel なしで指定されている場合はエラー
            if step.aggregate.is_some() {
                return Err(ValidationError::AggregateWithoutParallel {
                    step: step.name.clone(),
                });
            }

            // collect なしの step: ファセット参照が必要
            if step.collect.is_none() && !step.has_facet_refs() {
                return Err(ValidationError::MissingFacet {
                    step: step.name.clone(),
                });
            }

            // rules の遷移先チェック
            for rule in &step.rules {
                if !step_names.contains(rule.next.as_str()) {
                    return Err(ValidationError::UnknownNextStep {
                        step: step.name.clone(),
                        next: rule.next.clone(),
                    });
                }
            }

            // pass_output_from の参照先 step 名が存在するか検証
            if let Some(ref refs) = step.pass_output_from {
                for r in refs {
                    if !step_names.contains(r.as_str()) {
                        return Err(ValidationError::UnknownOutputFrom {
                            step: step.name.clone(),
                            reference: r.clone(),
                        });
                    }
                }
            }

            // collect.from の参照先 step 名が存在するか検証
            if let Some(ref collect) = step.collect {
                for r in &collect.from {
                    if !step_names.contains(r.as_str()) {
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

        // 次のステップのvalidationで使えるよう、このステップ名を追加
        preceding_step_names.insert(&step.name);
        // parallel blockの子step名も追加（後続parallel blockの子stepから参照可能にするため）
        if let Some(ref children) = step.parallel {
            for child in children {
                preceding_step_names.insert(&child.name);
            }
        }
    }

    Ok(())
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
            collect: None,
            parallel: None,
            aggregate: None,
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
            collect: None,
            parallel: Some(children),
            aggregate,
        }
    }

    // ---- 既存テスト ----

    #[test]
    fn valid_workflow_passes() {
        let wf = make_workflow(vec![
            make_step("plan", StepMode::Interactive, vec![]),
            Step {
                cycle_guard: Some(CycleGuard { max_iterations: 3 }),
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
    fn invalid_transition_target_fails() {
        let wf = make_workflow(vec![Step {
            rules: vec![TransitionRule {
                r#match: "DONE".to_string(),
                next: "nonexistent".to_string(),
            }],
            ..make_step("plan", StepMode::Interactive, vec![])
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
    fn duplicate_step_names_fails() {
        let wf = make_workflow(vec![
            make_step("plan", StepMode::Interactive, vec![]),
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
    fn parallel_child_pass_output_from_subsequent_step_fails() {
        // parallel block より後に定義されたステップへの参照は拒否される
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
        let err = validate(&wf).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::UnknownOutputFrom { ref reference, .. } if reference == "report"
        ));
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
}
