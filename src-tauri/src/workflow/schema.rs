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
    #[serde(default)]
    pub mode: Option<StepMode>,
    #[serde(default)]
    pub policy: Option<String>,
    #[serde(default)]
    pub knowledge: Option<String>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub output_contract: Option<String>,
    #[serde(default)]
    pub rules: Vec<TransitionRule>,
    #[serde(default)]
    pub cycle_guard: Option<CycleGuard>,
    #[serde(default)]
    pub pass_previous_response: Option<bool>,
    #[serde(default)]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default)]
    pub inline_prompt: Option<String>,
    #[serde(default)]
    pub collect: Option<CollectConfig>,
    #[serde(default)]
    pub parallel: Option<Vec<ParallelStep>>,
    #[serde(default)]
    pub aggregate: Option<AggregateConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_cycle_for: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

fn has_any_facet_ref(
    policy: &Option<String>,
    knowledge: &Option<String>,
    instruction: &Option<String>,
    output_contract: &Option<String>,
) -> bool {
    policy.is_some() || knowledge.is_some() || instruction.is_some() || output_contract.is_some()
}

impl Step {
    pub fn has_facet_refs(&self) -> bool {
        has_any_facet_ref(
            &self.policy,
            &self.knowledge,
            &self.instruction,
            &self.output_contract,
        )
    }

    /// validation通過後の通常stepではSomeが保証される。
    /// parallel blockではpanicするため、呼び出し前にis_parallel_block()で確認すること。
    pub fn mode_unwrap(&self) -> &StepMode {
        self.mode
            .as_ref()
            .expect("mode is required for non-parallel steps")
    }

    /// このステップがparallel blockかどうかを返す。
    pub fn is_parallel_block(&self) -> bool {
        self.parallel.is_some()
    }
}

/// 並列ブロック内の子ステップ定義。
/// 通常Stepとは別型で、許可するフィールドを制限する。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParallelStep {
    pub name: String,
    pub mode: StepMode,
    #[serde(default)]
    pub policy: Option<String>,
    #[serde(default)]
    pub knowledge: Option<String>,
    #[serde(default)]
    pub instruction: Option<String>,
    #[serde(default)]
    pub output_contract: Option<String>,
    #[serde(default)]
    pub pass_previous_response: Option<bool>,
    #[serde(default)]
    pub pass_output_from: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

impl ParallelStep {
    pub fn has_facet_refs(&self) -> bool {
        has_any_facet_ref(
            &self.policy,
            &self.knowledge,
            &self.instruction,
            &self.output_contract,
        )
    }
}

/// 並列ブロック完了後の集約条件。
/// all_matchとany_matchは排他（どちらか一方を必須）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregateConfig {
    #[serde(default)]
    pub all_match: Option<String>,
    #[serde(default)]
    pub any_match: Option<String>,
    pub then: String,
    pub r#else: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_exhausted: Option<String>,
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
    #[serde(default)]
    pub is_running: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FacetSummary {
    pub key: String,
    pub kind: String,
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
    instruction: plan

  - name: implement
    mode: auto
    instruction: implement
    policy: coding

  - name: review
    mode: auto
    instruction: review
    policy: review
    rules:
      - match: NEEDS_FIX
        next: implement
      - match: LGTM
        next: report
    cycle_guard:
      max_iterations: 5

  - name: report
    mode: approval
    instruction: report
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.name, "plan-implement-review");
        assert_eq!(wf.description, "計画→実装→レビュー→修正ループ");
        assert!(!wf.builtin);
        assert_eq!(wf.steps.len(), 4);

        let plan = &wf.steps[0];
        assert_eq!(plan.name, "plan");
        assert_eq!(plan.mode, Some(StepMode::Interactive));
        assert_eq!(plan.instruction.as_deref(), Some("plan"));
        assert!(plan.rules.is_empty());
        assert!(plan.cycle_guard.is_none());

        let review = &wf.steps[2];
        assert_eq!(review.name, "review");
        assert_eq!(review.mode, Some(StepMode::Auto));
        assert_eq!(review.rules.len(), 2);
        assert_eq!(review.rules[0].r#match, "NEEDS_FIX");
        assert_eq!(review.rules[0].next, "implement");
        assert_eq!(review.rules[1].r#match, "LGTM");
        assert_eq!(review.rules[1].next, "report");
        assert_eq!(review.cycle_guard.as_ref().unwrap().max_iterations, 5);

        let report = &wf.steps[3];
        assert_eq!(report.mode, Some(StepMode::Approval));
    }

    #[test]
    fn parse_unknown_mode_fails() {
        let yaml = r#"
name: bad
description: bad workflow
steps:
  - name: step1
    mode: unknown
    instruction: test
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
    instruction: fix
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert!(wf.builtin);
    }

    #[test]
    fn parse_collect_step_without_facets() {
        let yaml = r#"
name: collect-test
description: collect step test
steps:
  - name: review_a
    mode: auto
    instruction: review
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
        assert!(!collect_step.has_facet_refs());
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
    instruction: implement
  - name: step_b
    mode: auto
    instruction: review
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
    instruction: plan
  - name: step_b
    mode: auto
    instruction: implement
  - name: step_c
    mode: auto
    instruction: review
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
    fn parse_step_with_facet_refs() {
        let yaml = r#"
name: facet-test
description: facet test
steps:
  - name: implement
    mode: auto
    policy: coding
    instruction: implement
    knowledge: architecture
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let step = &wf.steps[0];
        assert_eq!(step.policy.as_deref(), Some("coding"));
        assert_eq!(step.instruction.as_deref(), Some("implement"));
        assert_eq!(step.knowledge.as_deref(), Some("architecture"));
        assert_eq!(step.output_contract, None);
        assert!(step.has_facet_refs());
    }

    #[test]
    fn step_without_facet_refs() {
        let step = Step {
            name: "collect".to_string(),
            mode: Some(StepMode::Auto),
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            rules: vec![],
            cycle_guard: None,
            pass_previous_response: None,
            pass_output_from: None,
            inline_prompt: None,
            collect: Some(CollectConfig {
                from: vec!["a".to_string()],
                reduce: ReduceStrategy::Concat,
            }),
            parallel: None,
            aggregate: None,
            resets_cycle_for: None,
            model: None,
            permission: None,
        };
        assert!(!step.has_facet_refs());
    }

    #[test]
    fn parse_parallel_block() {
        let yaml = r#"
name: parallel-test
description: parallel block test
steps:
  - name: implement
    mode: auto
    instruction: implement
  - name: parallel-review
    parallel:
      - name: arch-review
        mode: auto
        policy: review
        instruction: architecture-review
      - name: security-review
        mode: auto
        policy: review
        instruction: security-review
    aggregate:
      all_match: "LGTM"
      then: report
      else: implement
  - name: report
    mode: auto
    instruction: report
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.steps.len(), 3);

        let parallel_step = &wf.steps[1];
        assert_eq!(parallel_step.name, "parallel-review");
        assert!(parallel_step.is_parallel_block());
        assert_eq!(parallel_step.mode, None);

        let children = parallel_step.parallel.as_ref().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "arch-review");
        assert_eq!(children[0].mode, StepMode::Auto);
        assert_eq!(children[0].policy.as_deref(), Some("review"));
        assert_eq!(
            children[0].instruction.as_deref(),
            Some("architecture-review")
        );
        assert_eq!(children[1].name, "security-review");

        let agg = parallel_step.aggregate.as_ref().unwrap();
        assert_eq!(agg.all_match.as_deref(), Some("LGTM"));
        assert!(agg.any_match.is_none());
        assert_eq!(agg.then, "report");
        assert_eq!(agg.r#else, "implement");
    }

    #[test]
    fn parallel_step_has_facet_refs() {
        let ps = ParallelStep {
            name: "review".to_string(),
            mode: StepMode::Auto,
            policy: Some("review".to_string()),
            knowledge: None,
            instruction: Some("review".to_string()),
            output_contract: None,
            pass_previous_response: None,
            pass_output_from: None,
            model: None,
            permission: None,
        };
        assert!(ps.has_facet_refs());

        let ps_no_facet = ParallelStep {
            name: "empty".to_string(),
            mode: StepMode::Auto,
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            pass_previous_response: None,
            pass_output_from: None,
            model: None,
            permission: None,
        };
        assert!(!ps_no_facet.has_facet_refs());
    }

    #[test]
    fn parse_cycle_guard_with_on_exhausted() {
        let yaml = r#"
name: exhausted-test
description: on_exhausted test
steps:
  - name: fix
    mode: auto
    instruction: fix
    rules:
      - match: ".*"
        next: review
    cycle_guard:
      max_iterations: 2
      on_exhausted: approval
  - name: review
    mode: auto
    instruction: review
  - name: approval
    mode: interactive
    instruction: approve
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let guard = wf.steps[0].cycle_guard.as_ref().unwrap();
        assert_eq!(guard.max_iterations, 2);
        assert_eq!(guard.on_exhausted.as_deref(), Some("approval"));
    }

    #[test]
    fn parse_cycle_guard_without_on_exhausted_defaults_to_none() {
        let yaml = r#"
name: default-test
description: default on_exhausted test
steps:
  - name: review
    mode: auto
    instruction: review
    cycle_guard:
      max_iterations: 5
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let guard = wf.steps[0].cycle_guard.as_ref().unwrap();
        assert_eq!(guard.max_iterations, 5);
        assert!(guard.on_exhausted.is_none());
    }

    #[test]
    fn parse_step_with_resets_cycle_for() {
        let yaml = r#"
name: reset-test
description: resets_cycle_for test
steps:
  - name: fix
    mode: auto
    instruction: fix
    cycle_guard:
      max_iterations: 3
  - name: approval
    mode: interactive
    instruction: approve
    resets_cycle_for:
      - fix
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.steps[1].resets_cycle_for, Some(vec!["fix".to_string()]));
        assert!(wf.steps[0].resets_cycle_for.is_none());
    }

    #[test]
    fn parse_step_with_inline_prompt() {
        let yaml = r#"
name: inline-test
description: inline prompt test
steps:
  - name: quick
    mode: auto
    inline_prompt: "Do a quick analysis"
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(
            wf.steps[0].inline_prompt.as_deref(),
            Some("Do a quick analysis")
        );
        assert!(!wf.steps[0].has_facet_refs());
    }

    #[test]
    fn parse_step_without_inline_prompt_defaults_to_none() {
        let yaml = r#"
name: no-inline-test
description: no inline prompt
steps:
  - name: step1
    mode: auto
    instruction: implement
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert!(wf.steps[0].inline_prompt.is_none());
    }

    #[test]
    fn parse_step_with_model_and_permission() {
        let yaml = r#"
name: model-test
description: model/permission test
steps:
  - name: plan
    mode: auto
    instruction: plan
    model: opus-4
    permission: plan
  - name: implement
    mode: auto
    instruction: implement
    model: codex-mini
    permission: bypassPermissions
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.steps[0].model.as_deref(), Some("opus-4"));
        assert_eq!(wf.steps[0].permission.as_deref(), Some("plan"));
        assert_eq!(wf.steps[1].model.as_deref(), Some("codex-mini"));
        assert_eq!(wf.steps[1].permission.as_deref(), Some("bypassPermissions"));
    }

    #[test]
    fn parse_step_without_model_permission_defaults_to_none() {
        let yaml = r#"
name: default-test
description: default test
steps:
  - name: step1
    mode: auto
    instruction: implement
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert!(wf.steps[0].model.is_none());
        assert!(wf.steps[0].permission.is_none());
    }

    #[test]
    fn parse_parallel_step_with_model_and_permission() {
        let yaml = r#"
name: parallel-model-test
description: parallel model/permission test
steps:
  - name: parallel-review
    parallel:
      - name: arch-review
        mode: auto
        policy: review
        instruction: architecture-review
        model: opus-4
        permission: plan
      - name: security-review
        mode: auto
        policy: review
        instruction: security-review
        model: codex-mini
        permission: bypassPermissions
    aggregate:
      all_match: "LGTM"
      then: parallel-review
      else: parallel-review
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let children = wf.steps[0].parallel.as_ref().unwrap();
        assert_eq!(children[0].model.as_deref(), Some("opus-4"));
        assert_eq!(children[0].permission.as_deref(), Some("plan"));
        assert_eq!(children[1].model.as_deref(), Some("codex-mini"));
        assert_eq!(children[1].permission.as_deref(), Some("bypassPermissions"));
    }

    #[test]
    fn parse_step_model_only() {
        let yaml = r#"
name: model-only-test
description: model only test
steps:
  - name: step1
    mode: auto
    instruction: implement
    model: haiku
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(wf.steps[0].model.as_deref(), Some("haiku"));
        assert!(wf.steps[0].permission.is_none());
    }

    #[test]
    fn parse_step_permission_only() {
        let yaml = r#"
name: permission-only-test
description: permission only test
steps:
  - name: step1
    mode: auto
    instruction: implement
    permission: plan
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        assert!(wf.steps[0].model.is_none());
        assert_eq!(wf.steps[0].permission.as_deref(), Some("plan"));
    }
}
