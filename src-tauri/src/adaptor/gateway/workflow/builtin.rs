use std::fmt;

use super::diagnostics;
use super::domain_mapping::workflow_definition_to_domain;
use super::facet::{self, FacetError, FacetKind};
use super::schema::{Summary, WorkflowDefinitionYaml};
use super::storage;
use crate::domain::workflow::validation::{self, ValidationError};

const BUILTIN_01_AUTHOR_SPEC: &str = include_str!("builtin/01_author-spec.yml");
const BUILTIN_02_IMPLEMENT_EXISTING_SPEC: &str =
    include_str!("builtin/02_implement-existing-spec.yml");
const BUILTIN_03_FULL_REVIEW: &str = include_str!("builtin/03_full-review.yml");
const BUILTIN_04_REVIEW_FIX_POLICY: &str = include_str!("builtin/04_review-fix-policy.yml");
const BUILTIN_04_REVIEW_FIX_POLICY_MANUAL: &str =
    include_str!("builtin/04_review-fix-policy-manual.yml");
const BUILTIN_05_REVIEW_FIX: &str = include_str!("builtin/05_review-fix.yml");
const BUILTIN_06_HANDLE_PR_REVIEW: &str = include_str!("builtin/06_handle-pr-review.yml");
const BUILTIN_06_HANDLE_PR_REVIEW_MANUAL: &str =
    include_str!("builtin/06_handle-pr-review-manual.yml");

struct BuiltinEntry {
    filename: &'static str,
    content: &'static str,
    description: &'static str,
}

const BUILTINS: &[BuiltinEntry] = &[
    BuiltinEntry {
        filename: "01_author-spec.yml",
        content: BUILTIN_01_AUTHOR_SPEC,
        description: "Issue、Story、または自由文RequestからRequirements・Behavior・Designを順番に作成し、3文書を1単位として検証・検討・修正を収束させた後、最後に人間が完成Specをレビューする。",
    },
    BuiltinEntry {
        filename: "02_implement-existing-spec.yml",
        content: BUILTIN_02_IMPLEMENT_EXISTING_SPEC,
        description: "既存Specを入力として、並列タスク分解・fanout実装・一括検証ゲートを全Task完了までループし、Human checkpointで承認する。",
    },
    BuiltinEntry {
        filename: "03_full-review.yml",
        content: BUILTIN_03_FULL_REVIEW,
        description: "既存Specを入力として、FullReview（6観点×2モデル）と検証を実行し、open Threadを提示して完了する。",
    },
    BuiltinEntry {
        filename: "04_review-fix-policy.yml",
        content: BUILTIN_04_REVIEW_FIX_POLICY,
        description: "open Review Threadごとに修正方針を決定し、方針間の整合性を検証して完了する。",
    },
    BuiltinEntry {
        filename: "04_review-fix-policy-manual.yml",
        content: BUILTIN_04_REVIEW_FIX_POLICY_MANUAL,
        description: "open Review Threadごとに修正方針を人間と逐一合意して決定し、方針間の整合性を検証してHuman checkpointで承認する。",
    },
    BuiltinEntry {
        filename: "05_review-fix.yml",
        content: BUILTIN_05_REVIEW_FIX,
        description: "決定済み方針に基づき修正計画の作成と実装を行い、open Threadが解消するまで最大5回繰り返して完了する。",
    },
    BuiltinEntry {
        filename: "06_handle-pr-review.yml",
        content: BUILTIN_06_HANDLE_PR_REVIEW,
        description: "現在のブランチに紐づくPRの未解決review commentを取り込み、方針整合性を確認して修正し、人間の確認後にcommit、push、replyを行う。",
    },
    BuiltinEntry {
        filename: "06_handle-pr-review-manual.yml",
        content: BUILTIN_06_HANDLE_PR_REVIEW_MANUAL,
        description: "現在のブランチに紐づくPRの未解決review commentを取り込み、修正・返信方針をThreadごとに人間と逐一合意して修正し、確認後にcommit、push、replyを行う。",
    },
];

/// builtin workflow の load パイプラインで発生しうるエラー。
///
/// `serde_saphyr::Error` は型を直接持たず、format 済み文字列を保持する
/// （`Display` で表示するだけのため）。`ValidationError` / `FacetError` は
/// 既存型を `source` フィールドにそのまま保持し、上位で patten match 可能。
///
/// `serde_saphyr::Error` / `ValidationError` の失敗は仕様上「同梱 YAML が壊れている」
/// 状態（CI で 100% 再現する）。`FacetError` は利用者環境の facets ディレクトリ
/// I/O に依存するため、起動時 panic ではなく上位への伝播で扱う。
#[derive(Debug)]
pub enum BuiltinError {
    YamlParse {
        filename: &'static str,
        message: String,
    },
    Diagnostics {
        filename: &'static str,
        diagnostics: Vec<diagnostics::DiagnosticItem>,
    },
    Validation {
        filename: &'static str,
        source: ValidationError,
    },
    FacetResolution {
        filename: &'static str,
        source: FacetError,
    },
}

impl fmt::Display for BuiltinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::YamlParse { filename, message } => {
                write!(f, "Invalid builtin workflow '{filename}': {message}")
            }
            Self::Diagnostics {
                filename,
                diagnostics,
            } => {
                let messages = diagnostics
                    .iter()
                    .map(|item| format!("{}: {}", item.code, item.message))
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "Invalid builtin workflow '{filename}': {messages}")
            }
            Self::Validation { filename, source } => write!(
                f,
                "Invalid builtin workflow '{filename}': {source}",
                filename = filename,
                source = source
            ),
            Self::FacetResolution { filename, source } => write!(
                f,
                "Failed to resolve facets for builtin workflow '{filename}': {source}",
                filename = filename,
                source = source
            ),
        }
    }
}

impl std::error::Error for BuiltinError {}

/// 共通 load パイプライン: parse → builtin flag 設定 → validation → facet 解決後参照検証。
///
/// [02] schema 境界: built-in と user-authored YAML の load 経路を統一する。
/// 解決済み facet 本文は gateway 側 read model として検証し、Workflow definition には
/// facet ref だけを残す。
///
/// `base_facets_dir` には builtin 用に `facets_base_dir()` を渡せば、user-side で
/// 上書きされた facet も拾える。fallback として builtin 同梱本文が `facet::load_facet`
/// 内で参照されるため、builtin workflow の facet 参照は必ず解決される。
///
/// `Result::Ok(None)` は「`name` に対応する builtin が存在しない」を表し、
/// load 失敗とは区別する。
pub fn load_builtin_workflow_resolved(
    name: &str,
) -> Result<Option<WorkflowDefinitionYaml>, BuiltinError> {
    let Some(entry) = BUILTINS
        .iter()
        .find(|e| e.filename.strip_suffix(".yml") == Some(name))
    else {
        return Ok(None);
    };
    let diagnosis = diagnostics::diagnose_workflow_source(entry.content, Some(name));
    if diagnosis.has_errors() {
        return Err(BuiltinError::Diagnostics {
            filename: entry.filename,
            diagnostics: diagnosis.diagnostics,
        });
    }
    let mut wf: WorkflowDefinitionYaml =
        diagnosis.workflow.ok_or_else(|| BuiltinError::YamlParse {
            filename: entry.filename,
            message: "workflow source could not be parsed".to_string(),
        })?;
    wf.builtin = true;
    validation::validate(&workflow_definition_to_domain(&wf)).map_err(|err| {
        BuiltinError::Validation {
            filename: entry.filename,
            source: err,
        }
    })?;
    let base_dir = facet::facets_base_dir();
    if let Err(err) = storage::resolve_and_validate_workflow_facets(&wf, &base_dir) {
        return Err(match err {
            storage::StorageError::FacetResolution(source) => BuiltinError::FacetResolution {
                filename: entry.filename,
                source,
            },
            storage::StorageError::Validation(source) => BuiltinError::Validation {
                filename: entry.filename,
                source,
            },
            storage::StorageError::Diagnostics(diagnostics) => BuiltinError::Diagnostics {
                filename: entry.filename,
                diagnostics,
            },
            other => BuiltinError::Validation {
                filename: entry.filename,
                source: validation::ValidationError::InvalidArtifactReference {
                    reference: entry.filename.to_string(),
                    kind: validation::InvalidArtifactReferenceKind::InvalidInputRef,
                    reason: other.to_string(),
                },
            },
        });
    }
    Ok(Some(wf))
}

/// 同梱 YAML を再 parse せず、`BUILTINS` メタデータから直接 `Summary` を組み立てる。
///
/// 旧実装は listing 経路でも YAML を `serde_saphyr::from_str` で parse して
/// `description` を取り出し、失敗時に `panic!` で落とす形だったが、
/// (1) `load_builtin_workflow_resolved` 経路と二重 parse になり、
/// (2) listing が同梱資産の壊れ方で panic するのは load パイプラインの
/// `BuiltinError` 設計と非対称、という問題があった。description を
/// `BuiltinEntry` のコンパイル時メタデータに移すことで両方を解消する。
pub fn list_builtin_workflows() -> Vec<Summary> {
    BUILTINS
        .iter()
        .map(|e| Summary {
            name: e
                .filename
                .strip_suffix(".yml")
                .unwrap_or(e.filename)
                .to_string(),
            description: e.description.to_string(),
            builtin: true,
            is_running: false,
        })
        .collect()
}

// --- Builtin facets ---

struct BuiltinFacetEntry {
    kind: FacetKind,
    key: &'static str,
    content: &'static str,
}

const BUILTIN_FACETS: &[BuiltinFacetEntry] = &[
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "coding",
        content: include_str!("builtin_facets/policies/coding.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "planning",
        content: include_str!("builtin_facets/policies/planning.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "reviewing",
        content: include_str!("builtin_facets/policies/reviewing.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "reporting",
        content: include_str!("builtin_facets/policies/reporting.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "triage",
        content: include_str!("builtin_facets/policies/triage.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "author-spec-governance",
        content: include_str!("builtin_facets/policies/author-spec-governance.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Knowledge,
        key: "releash-thread-cli",
        content: include_str!("builtin_facets/knowledge/releash-thread-cli.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Knowledge,
        key: "implement-task",
        content: include_str!("builtin_facets/knowledge/implement-task.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Knowledge,
        key: "spec-behavior",
        content: include_str!("builtin_facets/knowledge/spec-behavior.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Knowledge,
        key: "spec-design",
        content: include_str!("builtin_facets/knowledge/spec-design.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Knowledge,
        key: "spec-requirements",
        content: include_str!("builtin_facets/knowledge/spec-requirements.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-consider-spec",
        content: include_str!("builtin_facets/instructions/author-spec-consider-spec.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-final-review",
        content: include_str!("builtin_facets/instructions/author-spec-final-review.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-human-decision",
        content: include_str!("builtin_facets/instructions/author-spec-human-decision.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-intake",
        content: include_str!("builtin_facets/instructions/author-spec-intake.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-repair-spec",
        content: include_str!("builtin_facets/instructions/author-spec-repair-spec.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-validate-spec",
        content: include_str!("builtin_facets/instructions/author-spec-validate-spec.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-write-behavior",
        content: include_str!("builtin_facets/instructions/author-spec-write-behavior.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-write-design",
        content: include_str!("builtin_facets/instructions/author-spec-write-design.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "author-spec-write-requirements",
        content: include_str!("builtin_facets/instructions/author-spec-write-requirements.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "create_parallel_tasks",
        content: include_str!("builtin_facets/instructions/create_parallel_tasks.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "existing-spec-check-fix-policy-consistency",
        content: include_str!(
            "builtin_facets/instructions/existing-spec-check-fix-policy-consistency.md"
        ),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "fix-report",
        content: include_str!("builtin_facets/instructions/fix-report.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implementation-confirmation",
        content: include_str!("builtin_facets/instructions/implementation-confirmation.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "policy-confirmation",
        content: include_str!("builtin_facets/instructions/policy-confirmation.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "policy-report",
        content: include_str!("builtin_facets/instructions/policy-report.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review-report",
        content: include_str!("builtin_facets/instructions/review-report.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "existing-spec-correct-fix-policy",
        content: include_str!("builtin_facets/instructions/existing-spec-correct-fix-policy.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "existing-spec-create-fix-plan",
        content: include_str!("builtin_facets/instructions/existing-spec-create-fix-plan.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "existing-spec-correct-fix-policy-manual",
        content: include_str!(
            "builtin_facets/instructions/existing-spec-correct-fix-policy-manual.md"
        ),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "existing-spec-decide-fix-policy",
        content: include_str!("builtin_facets/instructions/existing-spec-decide-fix-policy.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "existing-spec-decide-fix-policy-manual",
        content: include_str!(
            "builtin_facets/instructions/existing-spec-decide-fix-policy-manual.md"
        ),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "existing-spec-resolve-request",
        content: include_str!("builtin_facets/instructions/existing-spec-resolve-request.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implement_fix_plan",
        content: include_str!("builtin_facets/instructions/implement_fix_plan.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implement_single_task",
        content: include_str!("builtin_facets/instructions/implement_single_task.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "verify_fixes",
        content: include_str!("builtin_facets/instructions/verify_fixes.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "verify_implementation",
        content: include_str!("builtin_facets/instructions/verify_implementation.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review-acceptance",
        content: include_str!("builtin_facets/instructions/review-acceptance.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review-structure",
        content: include_str!("builtin_facets/instructions/review-structure.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review-quality",
        content: include_str!("builtin_facets/instructions/review-quality.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review-test",
        content: include_str!("builtin_facets/instructions/review-test.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review-security",
        content: include_str!("builtin_facets/instructions/review-security.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "review-architecture",
        content: include_str!("builtin_facets/instructions/review-architecture.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "full-review-verify-and-classify",
        content: include_str!("builtin_facets/instructions/full-review-verify-and-classify.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "import_pr_review_comments",
        content: include_str!("builtin_facets/instructions/import_pr_review_comments.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "decide_pr_review_fix_policy",
        content: include_str!("builtin_facets/instructions/decide_pr_review_fix_policy.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "decide_pr_review_fix_policy_manual",
        content: include_str!("builtin_facets/instructions/decide_pr_review_fix_policy_manual.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "check_pr_review_fix_policy_consistency",
        content: include_str!(
            "builtin_facets/instructions/check_pr_review_fix_policy_consistency.md"
        ),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "correct_pr_review_fix_policy",
        content: include_str!("builtin_facets/instructions/correct_pr_review_fix_policy.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "correct_pr_review_fix_policy_manual",
        content: include_str!("builtin_facets/instructions/correct_pr_review_fix_policy_manual.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "create_pr_review_fix_plan",
        content: include_str!("builtin_facets/instructions/create_pr_review_fix_plan.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implement_pr_review_fix_plan",
        content: include_str!("builtin_facets/instructions/implement_pr_review_fix_plan.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "verify_pr_review_fixes",
        content: include_str!("builtin_facets/instructions/verify_pr_review_fixes.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "pr_review_confirmation",
        content: include_str!("builtin_facets/instructions/pr_review_confirmation.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "finalize_pr_review",
        content: include_str!("builtin_facets/instructions/finalize_pr_review.md"),
    },
];

pub fn get_builtin_facet(kind: FacetKind, key: &str) -> Option<&'static str> {
    BUILTIN_FACETS
        .iter()
        .find(|e| e.kind == kind && e.key == key)
        .map(|e| e.content)
}

pub fn list_builtin_facet_keys(kind: FacetKind) -> Vec<&'static str> {
    BUILTIN_FACETS
        .iter()
        .filter(|e| e.kind == kind)
        .map(|e| e.key)
        .collect()
}

pub fn is_builtin_workflow(name: &str) -> bool {
    BUILTINS
        .iter()
        .any(|e| e.filename.strip_suffix(".yml") == Some(name))
}

pub fn builtin_workflow_source(name: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .find(|e| e.filename.strip_suffix(".yml") == Some(name))
        .map(|e| e.content)
}

pub fn is_builtin_facet(kind: FacetKind, key: &str) -> bool {
    BUILTIN_FACETS
        .iter()
        .any(|e| e.kind == kind && e.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_review_import_requires_all_graphql_pages() {
        let instruction =
            get_builtin_facet(FacetKind::Instruction, "import_pr_review_comments").unwrap();

        assert!(instruction.contains("reviewThreads(first:, after: endCursor)"));
        assert!(instruction.contains("comments(first:, after: endCursor)"));
        assert!(instruction.contains("pageInfo.hasNextPage"));
        assert!(instruction.contains("pageInfo.endCursor"));
        assert!(instruction.contains("全件取得できない場合はArtifactを提出せず"));
        assert!(instruction.contains("Nodeを失敗扱いにする"));
    }

    #[test]
    fn full_review_uses_current_opus_model() {
        let source = builtin_workflow_source("03_full-review").unwrap();

        assert!(source.contains("model: claude-opus-5"));
        assert!(!source.contains("claude-opus-4-8"));
    }
}
