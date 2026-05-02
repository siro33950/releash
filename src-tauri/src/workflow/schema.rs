use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Step {
    pub name: String,
    pub mode: StepMode,
    #[serde(default)]
    pub prompt: Option<StepPrompt>,
    #[serde(default)]
    pub rules: Vec<TransitionRule>,
    #[serde(default)]
    pub cycle_guard: Option<CycleGuard>,
    #[serde(default)]
    pub pass_previous_response: Option<bool>,
    #[serde(default)]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default)]
    pub collect: Option<CollectConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StepPrompt {
    Inline(String),
    InlineObject(InlinePrompt),
    Template(TemplatePrompt),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InlinePrompt {
    pub inline: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TemplatePrompt {
    pub template: String,
}

#[cfg(test)]
impl StepPrompt {
    pub fn inline(prompt: impl Into<String>) -> Option<Self> {
        Some(Self::Inline(prompt.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StepMode {
    Auto,
    Approval,
    Interactive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransitionRule {
    pub r#match: String,
    pub next: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CycleGuard {
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CollectConfig {
    pub from: Vec<String>,
    pub reduce: ReduceStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReduceStrategy {
    Last,
    Concat,
    Grouped,
    AnyNeedsFix,
    AllPassed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_workflow() {
        let yaml = r#"
name: plan-implement-review
description: 計画→実装→レビュー→修正ループ

steps:
  - name: plan
    mode: interactive
    prompt:
      template: planner

  - name: implement
    mode: auto
    prompt:
      template: coder

  - name: review
    mode: auto
    prompt:
      template: reviewer
    rules:
      - match: NEEDS_FIX
        next: implement
      - match: LGTM
        next: report
    cycle_guard:
      max_iterations: 5

  - name: report
    mode: approval
    prompt:
      template: reporter
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.name, "plan-implement-review");
        assert_eq!(wf.description, "計画→実装→レビュー→修正ループ");
        assert!(!wf.builtin);
        assert_eq!(wf.steps.len(), 4);

        let plan = &wf.steps[0];
        assert_eq!(plan.name, "plan");
        assert_eq!(plan.mode, StepMode::Interactive);
        assert_eq!(
            plan.prompt,
            Some(StepPrompt::Template(TemplatePrompt {
                template: "planner".to_string()
            }))
        );
        assert!(plan.rules.is_empty());
        assert!(plan.cycle_guard.is_none());

        let review = &wf.steps[2];
        assert_eq!(review.name, "review");
        assert_eq!(review.mode, StepMode::Auto);
        assert_eq!(review.rules.len(), 2);
        assert_eq!(review.rules[0].r#match, "NEEDS_FIX");
        assert_eq!(review.rules[0].next, "implement");
        assert_eq!(review.rules[1].r#match, "LGTM");
        assert_eq!(review.rules[1].next, "report");
        assert_eq!(review.cycle_guard.as_ref().unwrap().max_iterations, 5);

        let report = &wf.steps[3];
        assert_eq!(report.mode, StepMode::Approval);
    }

    #[test]
    fn parse_unknown_mode_fails() {
        let yaml = r#"
name: bad
description: bad workflow
steps:
  - name: step1
    mode: unknown
    prompt:
      inline: test
"#;
        let result: Result<Workflow, _> = serde_saphyr::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_builtin_field() {
        let yaml = r#"
name: quick-fix
description: Quick fix workflow
builtin: true
steps:
  - name: fix
    mode: auto
    prompt:
      template: fixer
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert!(wf.builtin);
    }

    #[test]
    fn parse_legacy_inline_prompt_string() {
        let yaml = r#"
name: legacy
description: legacy string prompt
steps:
  - name: step1
    mode: auto
    prompt: Run tests
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(
            wf.steps[0].prompt,
            Some(StepPrompt::Inline("Run tests".to_string()))
        );
    }

    #[test]
    fn parse_collect_step_without_prompt() {
        let yaml = r#"
name: collect-test
description: collect step test
steps:
  - name: review_a
    mode: auto
    prompt: Review A
    rules:
      - match: LGTM
        next: collect_reviews
      - match: NEEDS_FIX
        next: collect_reviews
  - name: collect_reviews
    mode: auto
    collect:
      from:
        - review_a
      reduce: any_needs_fix
    rules:
      - match: NEEDS_FIX
        next: review_a
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let collect_step = &wf.steps[1];
        assert_eq!(collect_step.name, "collect_reviews");
        assert_eq!(collect_step.prompt, None);
        let collect = collect_step.collect.as_ref().unwrap();
        assert_eq!(collect.from, vec!["review_a".to_string()]);
        assert_eq!(collect.reduce, ReduceStrategy::AnyNeedsFix);
    }

    #[test]
    fn parse_step_with_pass_previous_response() {
        let yaml = r#"
name: pass-test
description: pass previous response test
steps:
  - name: step_a
    mode: auto
    prompt: Do A
  - name: step_b
    mode: auto
    prompt: Do B
    pass_previous_response: true
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.steps[1].pass_previous_response, Some(true));
        assert_eq!(wf.steps[0].pass_previous_response, None);
    }

    #[test]
    fn parse_step_with_pass_output_from() {
        let yaml = r#"
name: output-from-test
description: pass output from test
steps:
  - name: step_a
    mode: auto
    prompt: Do A
  - name: step_b
    mode: auto
    prompt: Do B
  - name: step_c
    mode: auto
    prompt: Do C
    pass_output_from:
      - step_a
      - step_b
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(
            wf.steps[2].pass_output_from,
            Some(vec!["step_a".to_string(), "step_b".to_string()])
        );
    }

    #[test]
    fn parse_inline_prompt_object() {
        let yaml = r#"
name: inline-object
description: explicit inline prompt
steps:
  - name: step1
    mode: auto
    prompt:
      inline: Run tests
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(
            wf.steps[0].prompt,
            Some(StepPrompt::InlineObject(InlinePrompt {
                inline: "Run tests".to_string()
            }))
        );
    }
}
