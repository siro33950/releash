use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptTemplate {
    pub name: String,
    pub description: String,
    pub content: String,
    #[serde(default)]
    pub variables: Vec<PromptVariable>,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptVariable {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub default: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prompt_template_with_variables() {
        let yaml = r#"
name: fixer
description: バグ修正を行うプロンプト
content: |
  あなたは{{project_name}}のバグ修正担当です。
  以下の問題を修正してください。
variables:
  - name: project_name
    description: プロジェクト名
    default: my-project
"#;
        let tpl: PromptTemplate = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(tpl.name, "fixer");
        assert_eq!(tpl.description, "バグ修正を行うプロンプト");
        assert!(tpl.content.contains("{{project_name}}"));
        assert_eq!(tpl.variables.len(), 1);
        assert_eq!(tpl.variables[0].name, "project_name");
        assert_eq!(tpl.variables[0].default.as_deref(), Some("my-project"));
        assert!(!tpl.builtin);
    }

    #[test]
    fn parse_prompt_template_without_variables() {
        let yaml = r#"
name: reporter
description: レポート生成
content: 作業結果をレポートしてください。
"#;
        let tpl: PromptTemplate = serde_saphyr::from_str(yaml).unwrap();
        assert_eq!(tpl.name, "reporter");
        assert!(tpl.variables.is_empty());
        assert!(!tpl.builtin);
    }

    #[test]
    fn parse_builtin_prompt_template() {
        let yaml = r#"
name: planner
description: 計画立案
builtin: true
content: 計画を立ててください。
"#;
        let tpl: PromptTemplate = serde_saphyr::from_str(yaml).unwrap();
        assert!(tpl.builtin);
    }
}
