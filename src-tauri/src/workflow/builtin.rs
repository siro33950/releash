use super::schema::Workflow;
use super::storage;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum BuiltinInitError {
    Io(std::io::Error),
    Parse {
        filename: String,
        source: Box<serde_saphyr::Error>,
    },
    Storage(Box<storage::StorageError>),
}

impl fmt::Display for BuiltinInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/Oエラー: {e}"),
            Self::Parse { filename, source } => {
                write!(f, "ビルトインのパース失敗 ({filename}): {source}")
            }
            Self::Storage(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BuiltinInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Parse { source, .. } => Some(source.as_ref()),
            Self::Storage(e) => Some(e.as_ref()),
        }
    }
}

impl From<storage::StorageError> for BuiltinInitError {
    fn from(e: storage::StorageError) -> Self {
        Self::Storage(Box::new(e))
    }
}

impl Serialize for BuiltinInitError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

const BUILTIN_QUICK_FIX: &str = include_str!("builtin/quick-fix.yml");
const BUILTIN_PLAN_IMPLEMENT_REVIEW: &str = include_str!("builtin/plan-implement-review.yml");
const BUILTIN_TRACE_TEST: &str = include_str!("builtin/trace-test.yml");

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
    BuiltinEntry {
        filename: "trace-test.yml",
        content: BUILTIN_TRACE_TEST,
    },
];

fn init_builtins<T: DeserializeOwned>(
    dir: &Path,
    entries: &[BuiltinEntry],
) -> Result<(), BuiltinInitError> {
    storage::ensure_dir(dir)?;

    for entry in entries {
        let file_path = dir.join(entry.filename);
        if file_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(&file_path) {
                if existing == entry.content {
                    continue;
                }
            }
        }

        let _: T = serde_saphyr::from_str(entry.content).map_err(|e| BuiltinInitError::Parse {
            filename: entry.filename.to_string(),
            source: Box::new(e),
        })?;

        std::fs::write(&file_path, entry.content).map_err(BuiltinInitError::Io)?;
    }

    Ok(())
}

pub fn init_builtin_workflows(dir: &Path) -> Result<(), BuiltinInitError> {
    init_builtins::<Workflow>(dir, BUILTINS)
}

// --- Builtin facets ---

use super::facet::FacetKind;

struct BuiltinFacetEntry {
    kind: FacetKind,
    key: &'static str,
    content: &'static str,
}

const BUILTIN_FACETS: &[BuiltinFacetEntry] = &[
    BuiltinFacetEntry {
        kind: FacetKind::Persona,
        key: "planner",
        content: include_str!("builtin_facets/personas/planner.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Persona,
        key: "coder",
        content: include_str!("builtin_facets/personas/coder.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Persona,
        key: "reviewer",
        content: include_str!("builtin_facets/personas/reviewer.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "coding",
        content: include_str!("builtin_facets/policies/coding.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "review",
        content: include_str!("builtin_facets/policies/review.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Knowledge,
        key: "test-context",
        content: include_str!("builtin_facets/knowledge/test-context.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan",
        content: include_str!("builtin_facets/instructions/plan.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implement",
        content: include_str!("builtin_facets/instructions/implement.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review",
        content: include_str!("builtin_facets/instructions/review.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "fix",
        content: include_str!("builtin_facets/instructions/fix.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "verify",
        content: include_str!("builtin_facets/instructions/verify.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "report",
        content: include_str!("builtin_facets/instructions/report.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "test-step",
        content: include_str!("builtin_facets/instructions/test-step.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::OutputContract,
        key: "review-verdict",
        content: include_str!("builtin_facets/output_contracts/review-verdict.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::OutputContract,
        key: "fix-result",
        content: include_str!("builtin_facets/output_contracts/fix-result.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::OutputContract,
        key: "spec-file-path",
        content: include_str!("builtin_facets/output_contracts/spec-file-path.md"),
    },
];

pub fn init_builtin_facets(base_dir: &Path) -> Result<(), BuiltinInitError> {
    for entry in BUILTIN_FACETS {
        let dir = base_dir.join(entry.kind.dir_name());
        storage::ensure_dir(&dir)?;
        let file_path = dir.join(format!("{}.md", entry.key));
        if file_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(&file_path) {
                if existing == entry.content {
                    continue;
                }
            }
        }
        std::fs::write(&file_path, entry.content).map_err(BuiltinInitError::Io)?;
    }
    Ok(())
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
        assert!(dir.join("trace-test.yml").exists());

        let wf: Workflow =
            serde_saphyr::from_str(&std::fs::read_to_string(dir.join("quick-fix.yml")).unwrap())
                .unwrap();
        assert_eq!(wf.name, "quick-fix");
        assert!(wf.builtin);
    }

    #[test]
    fn init_overwrites_stale_builtin() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        init_builtin_workflows(dir).unwrap();

        // 旧版の内容を書き込み（バンドル版と異なる）
        let stale_content = r#"name: quick-fix
description: 旧版
builtin: true
steps:
  - name: old-step
    mode: auto
    instruction: fix
"#;
        std::fs::write(dir.join("quick-fix.yml"), stale_content).unwrap();

        // 再初期化でバンドル版に上書きされる
        init_builtin_workflows(dir).unwrap();

        let content = std::fs::read_to_string(dir.join("quick-fix.yml")).unwrap();
        assert!(!content.contains("旧版"));
    }

    #[test]
    fn init_skips_if_content_unchanged() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();

        init_builtin_workflows(dir).unwrap();
        let mtime1 = std::fs::metadata(dir.join("quick-fix.yml"))
            .unwrap()
            .modified()
            .unwrap();

        // 少し待って再初期化（内容同一ならスキップ）
        std::thread::sleep(std::time::Duration::from_millis(50));
        init_builtin_workflows(dir).unwrap();
        let mtime2 = std::fs::metadata(dir.join("quick-fix.yml"))
            .unwrap()
            .modified()
            .unwrap();

        assert_eq!(mtime1, mtime2);
    }

    #[test]
    fn init_creates_dir_if_missing() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("sub").join("workflows");

        init_builtin_workflows(&dir).unwrap();

        assert!(dir.join("quick-fix.yml").exists());
    }

    // --- Builtin facet tests ---

    #[test]
    fn init_creates_builtin_facets_in_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        init_builtin_facets(dir).unwrap();

        assert!(dir.join("personas/planner.md").exists());
        assert!(dir.join("personas/coder.md").exists());
        assert!(dir.join("personas/reviewer.md").exists());
        assert!(dir.join("policies/coding.md").exists());
        assert!(dir.join("policies/review.md").exists());
        assert!(dir.join("instructions/plan.md").exists());
        assert!(dir.join("instructions/implement.md").exists());
        assert!(dir.join("instructions/review.md").exists());
        assert!(dir.join("instructions/fix.md").exists());
        assert!(dir.join("instructions/verify.md").exists());
        assert!(dir.join("instructions/report.md").exists());
        assert!(dir.join("instructions/test-step.md").exists());
        assert!(dir.join("knowledge/test-context.md").exists());
        assert!(dir.join("output_contracts/review-verdict.md").exists());
        assert!(dir.join("output_contracts/fix-result.md").exists());
        assert!(dir.join("output_contracts/spec-file-path.md").exists());
    }

    #[test]
    fn init_builtin_facets_overwrites_stale() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        init_builtin_facets(dir).unwrap();

        std::fs::write(dir.join("personas/coder.md"), "旧版 persona").unwrap();
        init_builtin_facets(dir).unwrap();

        let content = std::fs::read_to_string(dir.join("personas/coder.md")).unwrap();
        assert!(!content.contains("旧版"));
    }
}
