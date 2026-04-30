use super::prompt_schema::PromptTemplate;
use super::schema::Workflow;
use super::storage;
use serde::de::DeserializeOwned;
use std::path::Path;

const BUILTIN_QUICK_FIX: &str = include_str!("builtin/quick-fix.yml");
const BUILTIN_PLAN_IMPLEMENT_REVIEW: &str = include_str!("builtin/plan-implement-review.yml");

const BUILTIN_PROMPT_FIXER: &str = include_str!("builtin_prompts/fixer.yml");
const BUILTIN_PROMPT_VERIFIER: &str = include_str!("builtin_prompts/verifier.yml");
const BUILTIN_PROMPT_PLANNER: &str = include_str!("builtin_prompts/planner.yml");
const BUILTIN_PROMPT_CODER: &str = include_str!("builtin_prompts/coder.yml");
const BUILTIN_PROMPT_REVIEWER: &str = include_str!("builtin_prompts/reviewer.yml");
const BUILTIN_PROMPT_REPORTER: &str = include_str!("builtin_prompts/reporter.yml");

struct BuiltinEntry {
    filename: &'static str,
    content: &'static str,
}

const BUILTINS: &[BuiltinEntry] = &[
    BuiltinEntry {
        filename: "quick-fix.yml",
        content: BUILTIN_QUICK_FIX,
    },
    BuiltinEntry {
        filename: "plan-implement-review.yml",
        content: BUILTIN_PLAN_IMPLEMENT_REVIEW,
    },
];

const BUILTIN_PROMPTS: &[BuiltinEntry] = &[
    BuiltinEntry {
        filename: "fixer.yml",
        content: BUILTIN_PROMPT_FIXER,
    },
    BuiltinEntry {
        filename: "verifier.yml",
        content: BUILTIN_PROMPT_VERIFIER,
    },
    BuiltinEntry {
        filename: "planner.yml",
        content: BUILTIN_PROMPT_PLANNER,
    },
    BuiltinEntry {
        filename: "coder.yml",
        content: BUILTIN_PROMPT_CODER,
    },
    BuiltinEntry {
        filename: "reviewer.yml",
        content: BUILTIN_PROMPT_REVIEWER,
    },
    BuiltinEntry {
        filename: "reporter.yml",
        content: BUILTIN_PROMPT_REPORTER,
    },
];

fn init_builtins<T: DeserializeOwned>(
    dir: &Path,
    entries: &[BuiltinEntry],
    label: &str,
) -> Result<(), String> {
    storage::ensure_dir(dir)?;

    for entry in entries {
        let file_path = dir.join(entry.filename);
        if file_path.exists() {
            continue;
        }

        let _: T = serde_saphyr::from_str(entry.content)
            .map_err(|e| format!("ビルトイン{label}のパース失敗 ({}): {e}", entry.filename))?;

        std::fs::write(&file_path, entry.content)
            .map_err(|e| format!("ビルトイン{label}の書き出し失敗 ({}): {e}", entry.filename))?;
    }

    Ok(())
}

pub fn init_builtin_workflows(dir: &Path) -> Result<(), String> {
    init_builtins::<Workflow>(dir, BUILTINS, "ワークフロー")
}

pub fn init_builtin_prompts(dir: &Path) -> Result<(), String> {
    init_builtins::<PromptTemplate>(dir, BUILTIN_PROMPTS, "プロンプト")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_builtins_in_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        init_builtin_workflows(dir).unwrap();

        assert!(dir.join("quick-fix.yml").exists());
        assert!(dir.join("plan-implement-review.yml").exists());

        let wf: Workflow =
            serde_saphyr::from_str(&std::fs::read_to_string(dir.join("quick-fix.yml")).unwrap())
                .unwrap();
        assert_eq!(wf.name, "quick-fix");
        assert!(wf.builtin);
    }

    #[test]
    fn init_does_not_overwrite_existing() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        init_builtin_workflows(dir).unwrap();

        let custom_content = r#"name: quick-fix
description: ユーザーがカスタマイズ済み
builtin: true
steps:
  - name: custom-step
    mode: auto
    prompt: custom
"#;
        std::fs::write(dir.join("quick-fix.yml"), custom_content).unwrap();

        init_builtin_workflows(dir).unwrap();

        let content = std::fs::read_to_string(dir.join("quick-fix.yml")).unwrap();
        assert!(content.contains("ユーザーがカスタマイズ済み"));
    }

    #[test]
    fn init_creates_dir_if_missing() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("sub").join("workflows");

        init_builtin_workflows(&dir).unwrap();

        assert!(dir.join("quick-fix.yml").exists());
    }

    // --- Builtin prompt tests ---

    use crate::workflow::prompt_schema::PromptTemplate;

    #[test]
    fn init_creates_builtin_prompts_in_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        init_builtin_prompts(dir).unwrap();

        assert!(dir.join("fixer.yml").exists());
        assert!(dir.join("verifier.yml").exists());
        assert!(dir.join("planner.yml").exists());
        assert!(dir.join("coder.yml").exists());
        assert!(dir.join("reviewer.yml").exists());
        assert!(dir.join("reporter.yml").exists());

        let tpl: PromptTemplate =
            serde_saphyr::from_str(&std::fs::read_to_string(dir.join("coder.yml")).unwrap())
                .unwrap();
        assert_eq!(tpl.name, "coder");
        assert!(tpl.builtin);
    }

    #[test]
    fn init_builtin_prompts_does_not_overwrite_existing() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        init_builtin_prompts(dir).unwrap();

        let custom_content = r#"name: coder
description: ユーザーがカスタマイズ済み
builtin: true
content: カスタムプロンプト
"#;
        std::fs::write(dir.join("coder.yml"), custom_content).unwrap();

        init_builtin_prompts(dir).unwrap();

        let content = std::fs::read_to_string(dir.join("coder.yml")).unwrap();
        assert!(content.contains("ユーザーがカスタマイズ済み"));
    }
}
