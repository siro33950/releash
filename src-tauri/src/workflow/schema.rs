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
    pub prompt: String,
    #[serde(default)]
    pub rules: Vec<TransitionRule>,
    #[serde(default)]
    pub cycle_guard: Option<CycleGuard>,
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
    prompt: planner

  - name: implement
    mode: auto
    prompt: coder

  - name: review
    mode: auto
    prompt: reviewer
    rules:
      - match: NEEDS_FIX
        next: implement
      - match: LGTM
        next: report
    cycle_guard:
      max_iterations: 5

  - name: report
    mode: approval
    prompt: reporter
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.name, "plan-implement-review");
        assert_eq!(wf.description, "計画→実装→レビュー→修正ループ");
        assert!(!wf.builtin);
        assert_eq!(wf.steps.len(), 4);

        let plan = &wf.steps[0];
        assert_eq!(plan.name, "plan");
        assert_eq!(plan.mode, StepMode::Interactive);
        assert_eq!(plan.prompt, "planner");
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
    prompt: test
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
    prompt: fixer
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert!(wf.builtin);
    }
}
