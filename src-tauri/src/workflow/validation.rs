use super::prompt_schema::PromptTemplate;
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
    EmptyTemplateContent,
    DuplicateVariable { name: String },
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
            Self::EmptyTemplateContent => {
                write!(f, "プロンプトテンプレートの内容が空です")
            }
            Self::DuplicateVariable { name } => {
                write!(f, "変数名 '{name}' が重複しています")
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

pub fn validate_prompt_template(template: &PromptTemplate) -> Result<(), ValidationError> {
    validate_name(&template.name)?;

    if template.content.trim().is_empty() {
        return Err(ValidationError::EmptyTemplateContent);
    }

    let mut var_names = HashSet::new();
    for var in &template.variables {
        if !var_names.insert(var.name.as_str()) {
            return Err(ValidationError::DuplicateVariable {
                name: var.name.clone(),
            });
        }
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
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::{CycleGuard, Step, StepMode, StepPrompt, TransitionRule};

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
                prompt: StepPrompt::inline("planner"),
                rules: vec![],
                cycle_guard: None,
            },
            Step {
                name: "implement".to_string(),
                mode: StepMode::Auto,
                prompt: StepPrompt::inline("coder"),
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
            prompt: StepPrompt::inline("planner"),
            rules: vec![TransitionRule {
                r#match: "DONE".to_string(),
                next: "nonexistent".to_string(),
            }],
            cycle_guard: None,
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
            Step {
                name: "plan".to_string(),
                mode: StepMode::Interactive,
                prompt: StepPrompt::inline("planner"),
                rules: vec![],
                cycle_guard: None,
            },
            Step {
                name: "plan".to_string(),
                mode: StepMode::Auto,
                prompt: StepPrompt::inline("coder"),
                rules: vec![],
                cycle_guard: None,
            },
        ]);
        let result = validate(&wf);
        assert!(matches!(
            result.unwrap_err(),
            ValidationError::DuplicateStep { ref name } if name == "plan"
        ));
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
        assert!(matches!(
            validate_prompt_template(&tpl).unwrap_err(),
            ValidationError::EmptyTemplateContent
        ));
    }

    #[test]
    fn prompt_template_whitespace_only_content_fails() {
        let cases = ["   ", "\n", "\t", "  \n\t  "];
        for content in &cases {
            let tpl = make_prompt("fixer", content, vec![]);
            assert!(
                matches!(
                    validate_prompt_template(&tpl).unwrap_err(),
                    ValidationError::EmptyTemplateContent
                ),
                "whitespace-only content {:?} should fail validation",
                content
            );
        }
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
        assert!(matches!(
            validate_prompt_template(&tpl).unwrap_err(),
            ValidationError::DuplicateVariable { ref name } if name == "var1"
        ));
    }
}
