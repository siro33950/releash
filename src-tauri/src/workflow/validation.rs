use super::prompt_schema::PromptTemplate;
use super::schema::Workflow;
use std::collections::HashSet;

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("ワークフロー名が空です".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "ワークフロー名 '{name}' に使用できない文字が含まれています（英数字・ハイフン・アンダースコアのみ許可）"
        ));
    }
    Ok(())
}

pub fn validate_prompt_template(template: &PromptTemplate) -> Result<(), String> {
    validate_name(&template.name)?;

    if template.content.is_empty() {
        return Err("プロンプトテンプレートの内容が空です".to_string());
    }

    let mut var_names = HashSet::new();
    for var in &template.variables {
        if !var_names.insert(var.name.as_str()) {
            return Err(format!("変数名 '{}' が重複しています", var.name));
        }
    }

    Ok(())
}

pub fn validate(workflow: &Workflow) -> Result<(), String> {
    validate_name(&workflow.name)?;

    if workflow.steps.is_empty() {
        return Err("ワークフローにステップが定義されていません".to_string());
    }

    let mut step_names = HashSet::new();
    for step in &workflow.steps {
        if !step_names.insert(step.name.as_str()) {
            return Err(format!("ステップ名 '{}' が重複しています", step.name));
        }
    }

    for step in &workflow.steps {
        for rule in &step.rules {
            if !step_names.contains(rule.next.as_str()) {
                return Err(format!(
                    "ステップ '{}' のルールが存在しないステップ '{}' を参照しています",
                    step.name, rule.next
                ));
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

    #[test]
    fn valid_workflow_passes() {
        let wf = make_workflow(vec![
            Step {
                name: "plan".to_string(),
                mode: StepMode::Interactive,
                prompt: "planner".to_string(),
                rules: vec![],
                cycle_guard: None,
            },
            Step {
                name: "implement".to_string(),
                mode: StepMode::Auto,
                prompt: "coder".to_string(),
                rules: vec![TransitionRule {
                    r#match: "DONE".to_string(),
                    next: "plan".to_string(),
                }],
                cycle_guard: Some(CycleGuard { max_iterations: 3 }),
            },
        ]);
        assert!(validate(&wf).is_ok());
    }

    #[test]
    fn invalid_transition_target_fails() {
        let wf = make_workflow(vec![Step {
            name: "plan".to_string(),
            mode: StepMode::Interactive,
            prompt: "planner".to_string(),
            rules: vec![TransitionRule {
                r#match: "DONE".to_string(),
                next: "nonexistent".to_string(),
            }],
            cycle_guard: None,
        }]);
        let result = validate(&wf);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("存在しないステップ 'nonexistent'"));
    }

    #[test]
    fn empty_steps_fails() {
        let wf = make_workflow(vec![]);
        assert!(validate(&wf).is_err());
    }

    #[test]
    fn valid_name_passes() {
        assert!(validate_name("quick-fix").is_ok());
        assert!(validate_name("my_workflow_v2").is_ok());
        assert!(validate_name("test123").is_ok());
    }

    #[test]
    fn invalid_name_with_traversal() {
        assert!(validate_name("../evil").is_err());
        assert!(validate_name("foo/bar").is_err());
        assert!(validate_name("..").is_err());
    }

    #[test]
    fn invalid_name_with_special_chars() {
        assert!(validate_name("foo bar").is_err());
        assert!(validate_name("foo.yml").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn duplicate_step_names_fails() {
        let wf = make_workflow(vec![
            Step {
                name: "plan".to_string(),
                mode: StepMode::Interactive,
                prompt: "planner".to_string(),
                rules: vec![],
                cycle_guard: None,
            },
            Step {
                name: "plan".to_string(),
                mode: StepMode::Auto,
                prompt: "coder".to_string(),
                rules: vec![],
                cycle_guard: None,
            },
        ]);
        let result = validate(&wf);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("重複"));
    }

    // --- Prompt template validation tests ---

    use crate::workflow::prompt_schema::{PromptTemplate, PromptVariable};

    fn make_prompt(name: &str, content: &str, vars: Vec<PromptVariable>) -> PromptTemplate {
        PromptTemplate {
            name: name.to_string(),
            description: "テスト用".to_string(),
            content: content.to_string(),
            variables: vars,
            builtin: false,
        }
    }

    #[test]
    fn valid_prompt_template_passes() {
        let tpl = make_prompt(
            "fixer",
            "プロンプト内容",
            vec![PromptVariable {
                name: "project_name".to_string(),
                description: "プロジェクト名".to_string(),
                default: None,
            }],
        );
        assert!(validate_prompt_template(&tpl).is_ok());
    }

    #[test]
    fn prompt_template_empty_content_fails() {
        let tpl = make_prompt("fixer", "", vec![]);
        let result = validate_prompt_template(&tpl);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("内容が空"));
    }

    #[test]
    fn prompt_template_invalid_name_fails() {
        let tpl = make_prompt("../evil", "content", vec![]);
        assert!(validate_prompt_template(&tpl).is_err());
    }

    #[test]
    fn prompt_template_duplicate_variables_fails() {
        let tpl = make_prompt(
            "test",
            "content",
            vec![
                PromptVariable {
                    name: "var1".to_string(),
                    description: "a".to_string(),
                    default: None,
                },
                PromptVariable {
                    name: "var1".to_string(),
                    description: "b".to_string(),
                    default: None,
                },
            ],
        );
        let result = validate_prompt_template(&tpl);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("重複"));
    }
}
