use super::schema::Step;
use super::storage;
use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub enum FacetError {
    InvalidKey { key: String },
    NotFound { kind: FacetKind, key: String },
    Io(std::io::Error),
}

impl fmt::Display for FacetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey { key } => write!(
                f,
                "ファセットキー '{key}' が無効です（先頭は英数字、以降は英数字・ハイフン・アンダースコアのみ許可）"
            ),
            Self::NotFound { kind, key } => write!(
                f,
                "ファセット '{key}' ({}) が見つかりません",
                kind.dir_name()
            ),
            Self::Io(e) => write!(f, "I/Oエラー: {e}"),
        }
    }
}

impl std::error::Error for FacetError {}

impl From<std::io::Error> for FacetError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl Serialize for FacetError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacetKind {
    Persona,
    Policy,
    Knowledge,
    Instruction,
    OutputContract,
}

impl FacetKind {
    pub fn dir_name(&self) -> &str {
        match self {
            Self::Persona => "personas",
            Self::Policy => "policies",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instructions",
            Self::OutputContract => "output_contracts",
        }
    }
}

#[derive(Debug)]
pub struct ComposedPrompt {
    pub system_prompt: Option<String>,
    pub user_message: String,
}

pub fn validate_facet_key(key: &str) -> Result<(), FacetError> {
    if key.is_empty() {
        return Err(FacetError::InvalidKey {
            key: key.to_string(),
        });
    }
    let mut chars = key.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err(FacetError::InvalidKey {
            key: key.to_string(),
        });
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(FacetError::InvalidKey {
                key: key.to_string(),
            });
        }
    }
    Ok(())
}

pub fn load_facet(kind: FacetKind, key: &str, base_dir: &Path) -> Result<String, FacetError> {
    validate_facet_key(key)?;
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    if !path.exists() {
        return Err(FacetError::NotFound {
            kind,
            key: key.to_string(),
        });
    }
    Ok(fs::read_to_string(&path)?)
}

pub fn save_facet(
    kind: FacetKind,
    key: &str,
    content: &str,
    base_dir: &Path,
) -> Result<(), FacetError> {
    validate_facet_key(key)?;
    let dir = base_dir.join(kind.dir_name());
    storage::ensure_dir(&dir).map_err(|e| FacetError::Io(std::io::Error::other(e.to_string())))?;
    let path = dir.join(format!("{key}.md"));
    fs::write(&path, content)?;
    Ok(())
}

pub fn delete_facet(kind: FacetKind, key: &str, base_dir: &Path) -> Result<(), FacetError> {
    validate_facet_key(key)?;
    let path = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    if !path.exists() {
        return Err(FacetError::NotFound {
            kind,
            key: key.to_string(),
        });
    }
    fs::remove_file(&path)?;
    Ok(())
}

pub fn list_facets(kind: FacetKind, base_dir: &Path) -> Result<Vec<String>, FacetError> {
    let dir = base_dir.join(kind.dir_name());
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut keys = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                keys.push(stem.to_string());
            }
        }
    }
    keys.sort();
    Ok(keys)
}

pub fn compose_facets(step: &Step, base_dir: &Path) -> Result<ComposedPrompt, FacetError> {
    let system_prompt = match &step.persona {
        Some(key) => Some(load_facet(FacetKind::Persona, key, base_dir)?),
        None => None,
    };

    let mut parts: Vec<String> = Vec::new();

    if let Some(key) = &step.knowledge {
        parts.push(load_facet(FacetKind::Knowledge, key, base_dir)?);
    }
    if let Some(key) = &step.instruction {
        parts.push(load_facet(FacetKind::Instruction, key, base_dir)?);
    }
    if let Some(key) = &step.output_contract {
        parts.push(load_facet(FacetKind::OutputContract, key, base_dir)?);
    }
    if let Some(key) = &step.policy {
        parts.push(load_facet(FacetKind::Policy, key, base_dir)?);
    }

    Ok(ComposedPrompt {
        system_prompt,
        user_message: parts.join("\n\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::StepMode;
    use tempfile::TempDir;

    fn make_facet_step(
        persona: Option<&str>,
        policy: Option<&str>,
        knowledge: Option<&str>,
        instruction: Option<&str>,
        output_contract: Option<&str>,
    ) -> Step {
        Step {
            name: "test".to_string(),
            mode: StepMode::Auto,
            persona: persona.map(String::from),
            policy: policy.map(String::from),
            knowledge: knowledge.map(String::from),
            instruction: instruction.map(String::from),
            output_contract: output_contract.map(String::from),
            rules: vec![],
            cycle_guard: None,
            pass_previous_response: None,
            pass_output_from: None,
            collect: None,
        }
    }

    fn setup_facet_files(dir: &Path) {
        let personas = dir.join("personas");
        let policies = dir.join("policies");
        let knowledge = dir.join("knowledge");
        let instructions = dir.join("instructions");
        let output_contracts = dir.join("output_contracts");
        for d in [
            &personas,
            &policies,
            &knowledge,
            &instructions,
            &output_contracts,
        ] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(personas.join("coder.md"), "You are a coder.").unwrap();
        fs::write(personas.join("reviewer.md"), "You are a reviewer.").unwrap();
        fs::write(policies.join("coding.md"), "Follow best practices.").unwrap();
        fs::write(knowledge.join("architecture.md"), "The system uses Tauri.").unwrap();
        fs::write(instructions.join("implement.md"), "Implement the feature.").unwrap();
        fs::write(output_contracts.join("plan-doc.md"), "Output as markdown.").unwrap();
    }

    // --- validate_facet_key ---

    #[test]
    fn valid_keys() {
        assert!(validate_facet_key("coder").is_ok());
        assert!(validate_facet_key("my-facet").is_ok());
        assert!(validate_facet_key("test_123").is_ok());
        assert!(validate_facet_key("a").is_ok());
        assert!(validate_facet_key("A1-b_c").is_ok());
    }

    #[test]
    fn invalid_keys() {
        assert!(validate_facet_key("").is_err());
        assert!(validate_facet_key("-start").is_err());
        assert!(validate_facet_key("_start").is_err());
        assert!(validate_facet_key("a/b").is_err());
        assert!(validate_facet_key("a b").is_err());
        assert!(validate_facet_key("../evil").is_err());
    }

    // --- load_facet ---

    #[test]
    fn load_existing_facet() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());
        let content = load_facet(FacetKind::Persona, "coder", tmp.path()).unwrap();
        assert_eq!(content, "You are a coder.");
    }

    #[test]
    fn load_missing_facet_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = load_facet(FacetKind::Persona, "unknown", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    #[test]
    fn load_facet_with_invalid_key() {
        let tmp = TempDir::new().unwrap();
        let result = load_facet(FacetKind::Persona, "../evil", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::InvalidKey { .. }));
    }

    // --- save_facet ---

    #[test]
    fn save_new_facet() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Persona, "new-one", "content", tmp.path()).unwrap();
        let path = tmp.path().join("personas/new-one.md");
        assert!(path.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "content");
    }

    #[test]
    fn save_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Persona, "test", "v1", tmp.path()).unwrap();
        save_facet(FacetKind::Persona, "test", "v2", tmp.path()).unwrap();
        let content = load_facet(FacetKind::Persona, "test", tmp.path()).unwrap();
        assert_eq!(content, "v2");
    }

    #[test]
    fn save_with_invalid_key() {
        let tmp = TempDir::new().unwrap();
        let result = save_facet(FacetKind::Persona, "", "content", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::InvalidKey { .. }));
    }

    // --- delete_facet ---

    #[test]
    fn delete_existing_facet() {
        let tmp = TempDir::new().unwrap();
        save_facet(FacetKind::Persona, "deleteme", "content", tmp.path()).unwrap();
        delete_facet(FacetKind::Persona, "deleteme", tmp.path()).unwrap();
        assert!(!tmp.path().join("personas/deleteme.md").exists());
    }

    #[test]
    fn delete_missing_facet_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = delete_facet(FacetKind::Persona, "nope", tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    // --- list_facets ---

    #[test]
    fn list_facets_sorted() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());
        let keys = list_facets(FacetKind::Persona, tmp.path()).unwrap();
        assert_eq!(keys, vec!["coder", "reviewer"]);
    }

    #[test]
    fn list_facets_empty_dir() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("personas")).unwrap();
        let keys = list_facets(FacetKind::Persona, tmp.path()).unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn list_facets_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let keys = list_facets(FacetKind::Persona, tmp.path()).unwrap();
        assert!(keys.is_empty());
    }

    // --- compose_facets ---

    #[test]
    fn compose_all_facets() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let step = make_facet_step(
            Some("coder"),
            Some("coding"),
            Some("architecture"),
            Some("implement"),
            Some("plan-doc"),
        );
        let result = compose_facets(&step, tmp.path()).unwrap();

        assert_eq!(result.system_prompt.as_deref(), Some("You are a coder."));
        assert_eq!(
            result.user_message,
            "The system uses Tauri.\n\nImplement the feature.\n\nOutput as markdown.\n\nFollow best practices."
        );
    }

    #[test]
    fn compose_partial_facets() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let step = make_facet_step(Some("coder"), None, None, Some("implement"), None);
        let result = compose_facets(&step, tmp.path()).unwrap();

        assert_eq!(result.system_prompt.as_deref(), Some("You are a coder."));
        assert_eq!(result.user_message, "Implement the feature.");
    }

    #[test]
    fn compose_persona_only() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let step = make_facet_step(Some("coder"), None, None, None, None);
        let result = compose_facets(&step, tmp.path()).unwrap();

        assert_eq!(result.system_prompt.as_deref(), Some("You are a coder."));
        assert_eq!(result.user_message, "");
    }

    #[test]
    fn compose_no_persona() {
        let tmp = TempDir::new().unwrap();
        setup_facet_files(tmp.path());

        let step = make_facet_step(None, Some("coding"), None, Some("implement"), None);
        let result = compose_facets(&step, tmp.path()).unwrap();

        assert!(result.system_prompt.is_none());
        assert_eq!(
            result.user_message,
            "Implement the feature.\n\nFollow best practices."
        );
    }

    #[test]
    fn compose_with_missing_facet_returns_error() {
        let tmp = TempDir::new().unwrap();
        let step = make_facet_step(Some("nonexistent"), None, None, None, None);
        let result = compose_facets(&step, tmp.path());
        assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
    }

    // --- compose → render_facet_variables パイプライン結合テスト ---

    #[test]
    fn compose_and_render_pipeline() {
        use crate::workflow::engine::WorkflowEngine;

        let tmp = TempDir::new().unwrap();
        let personas = tmp.path().join("personas");
        let instructions = tmp.path().join("instructions");
        std::fs::create_dir_all(&personas).unwrap();
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(
            personas.join("coder.md"),
            "You are a coder for {{project_name}}.",
        )
        .unwrap();
        std::fs::write(
            instructions.join("impl.md"),
            "Task: {{task}}\nProject: {{project_name}}",
        )
        .unwrap();

        let step = make_facet_step(Some("coder"), None, None, Some("impl"), None);
        let composed = compose_facets(&step, tmp.path()).unwrap();

        let worktree_path = "/home/user/my-project";
        let task = Some("Fix the bug");

        let rendered_system = composed
            .system_prompt
            .map(|s| WorkflowEngine::render_facet_variables(&s, worktree_path, task));
        let rendered_user =
            WorkflowEngine::render_facet_variables(&composed.user_message, worktree_path, task);

        assert_eq!(rendered_system.unwrap(), "You are a coder for my-project.");
        assert_eq!(rendered_user, "Task: Fix the bug\nProject: my-project");
    }
}
