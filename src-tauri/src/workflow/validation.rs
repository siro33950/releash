use super::schema::Workflow;
use serde::Serialize;
use std::collections::HashSet;
use std::fmt;

#[derive(Debug)]
pub enum ValidationError {
    EmptyName,
    InvalidChars { name: String },
    EmptySteps,
    DuplicateStep { name: String },
    UnknownNextStep { step: String, next: String },
    MissingFacet { step: String },
    UnknownOutputFrom { step: String, reference: String },
    UnknownCollectFrom { step: String, reference: String },
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

    let mut step_names = HashSet::new();
    for step in &workflow.steps {
        if !step_names.insert(step.name.as_str()) {
            return Err(ValidationError::DuplicateStep {
                name: step.name.clone(),
            });
        }
    }

    for step in &workflow.steps {
        for rule in &step.rules {
            if !step_names.contains(rule.next.as_str()) {
                return Err(ValidationError::UnknownNextStep {
                    step: step.name.clone(),
                    next: rule.next.clone(),
                });
            }
        }

        // collect なしの step: ファセット参照が必要
        if step.collect.is_none() && !step.has_facet_refs() {
            return Err(ValidationError::MissingFacet {
                step: step.name.clone(),
            });
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
                        if rs.rules.is_empty() {
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::{CycleGuard, Step, StepMode, TransitionRule};

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
            mode,
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
        }
    }

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
        use crate::workflow::schema::{CollectConfig, ReduceStrategy};
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
        use crate::workflow::schema::{CollectConfig, ReduceStrategy};
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
}
