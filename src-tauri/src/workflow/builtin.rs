use std::fmt;

use super::facet::{self, FacetError, FacetKind};
use super::schema::{Summary, Workflow};
use super::validation::{self, ValidationError};

const BUILTIN_SPEC_DRIVEN_DEVELOPMENT: &str = include_str!("builtin/spec-driven-development.yml");

struct BuiltinEntry {
    filename: &'static str,
    content: &'static str,
    /// YAML 内の `description` フィールドと一致させる。`list_builtin_workflows` で
    /// YAML を再 parse せずに `Summary` を返すためのメタデータ。
    /// 同梱 YAML の description と乖離した場合は CI のテストで検知する
    /// (`builtin_entries_description_matches_yaml`)。
    description: &'static str,
}

const BUILTINS: &[BuiltinEntry] = &[BuiltinEntry {
    filename: "spec-driven-development.yml",
    content: BUILTIN_SPEC_DRIVEN_DEVELOPMENT,
    description: "Spec-driven development workflow (requirements → behavior → architecture → review → implement → review → approve)",
}];

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

/// 共通 load パイプライン: parse → builtin flag 設定 → validation → facet 解決。
///
/// [02] schema 境界: built-in と user-authored YAML の load 経路を統一する。
/// 解決済み facet (`ResolvedFacets`) を `NodeDefinition` / `ChildNodeDefinition` に
/// 流し込み、engine は未解決 ref を経由しない。
///
/// `base_facets_dir` には builtin 用に `facets_base_dir()` を渡せば、user-side で
/// 上書きされた facet も拾える。fallback として builtin 同梱本文が `facet::load_facet`
/// 内で参照されるため、builtin workflow の facet 参照は必ず解決される。
///
/// `Result::Ok(None)` は「`name` に対応する builtin が存在しない」を表し、
/// load 失敗とは区別する。
pub fn load_builtin_workflow_resolved(name: &str) -> Result<Option<Workflow>, BuiltinError> {
    let Some(entry) = BUILTINS
        .iter()
        .find(|e| e.filename.strip_suffix(".yml") == Some(name))
    else {
        return Ok(None);
    };
    let mut wf: Workflow =
        serde_saphyr::from_str(entry.content).map_err(|err| BuiltinError::YamlParse {
            filename: entry.filename,
            message: err.to_string(),
        })?;
    wf.builtin = true;
    validation::validate(&wf).map_err(|err| BuiltinError::Validation {
        filename: entry.filename,
        source: err,
    })?;
    let base_dir = facet::facets_base_dir();
    facet::resolve_workflow_facets(&mut wf, &base_dir).map_err(|err| {
        BuiltinError::FacetResolution {
            filename: entry.filename,
            source: err,
        }
    })?;
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
        key: "review",
        content: include_str!("builtin_facets/policies/review.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "planning",
        content: include_str!("builtin_facets/policies/planning.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Policy,
        key: "plan-review",
        content: include_str!("builtin_facets/policies/plan-review.md"),
    },
    // --- Spec-driven development workflow instructions ---
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-requirements",
        content: include_str!("builtin_facets/instructions/plan-requirements.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-behavior",
        content: include_str!("builtin_facets/instructions/plan-behavior.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-architecture",
        content: include_str!("builtin_facets/instructions/plan-architecture.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-review-completeness",
        content: include_str!("builtin_facets/instructions/plan-review-completeness.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-review-clarity",
        content: include_str!("builtin_facets/instructions/plan-review-clarity.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-review-security",
        content: include_str!("builtin_facets/instructions/plan-review-security.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-review-consistency",
        content: include_str!("builtin_facets/instructions/plan-review-consistency.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-fix",
        content: include_str!("builtin_facets/instructions/plan-fix.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-fix-policy",
        content: include_str!("builtin_facets/instructions/plan-fix-policy.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "plan-approval",
        content: include_str!("builtin_facets/instructions/plan-approval.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implement",
        content: include_str!("builtin_facets/instructions/implement.md"),
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
        key: "implement-fix",
        content: include_str!("builtin_facets/instructions/implement-fix.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implementation-fix-policy",
        content: include_str!("builtin_facets/instructions/implementation-fix-policy.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::Instruction,
        key: "implementation-approval",
        content: include_str!("builtin_facets/instructions/implementation-approval.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::OutputContract,
        key: "review-verdict",
        content: include_str!("builtin_facets/output_contracts/review-verdict.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::OutputContract,
        key: "spec-file-path",
        content: include_str!("builtin_facets/output_contracts/spec-file-path.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::OutputContract,
        key: "approved-fix-policy",
        content: include_str!("builtin_facets/output_contracts/approved-fix-policy.md"),
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

pub fn is_builtin_facet(kind: FacetKind, key: &str) -> bool {
    BUILTIN_FACETS
        .iter()
        .any(|e| e.kind == kind && e.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_builtin_workflow_resolved_returns_valid_workflow() {
        let wf = load_builtin_workflow_resolved("spec-driven-development")
            .expect("load pipeline must succeed for the bundled builtin")
            .expect("spec-driven-development must be found in BUILTINS");
        assert_eq!(wf.name, "spec-driven-development");
        assert!(wf.builtin);
    }

    #[test]
    fn load_builtin_workflow_resolved_returns_none_for_unknown() {
        let result = load_builtin_workflow_resolved("nonexistent")
            .expect("lookup itself must not fail for unknown");
        assert!(result.is_none());
    }

    #[test]
    fn list_builtin_workflows_returns_all() {
        let summaries = list_builtin_workflows();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "spec-driven-development");
        assert!(summaries[0].builtin);
    }

    /// `BuiltinEntry.description` (compile-time metadata) と
    /// 同梱 YAML の `description` フィールドが一致することを CI で担保する。
    /// `list_builtin_workflows` がメタデータ駆動で `Summary` を返す設計のため、
    /// YAML 側を書き換えてメタデータを忘れると利用者向け listing が乖離する。
    #[test]
    fn builtin_entries_description_matches_yaml() {
        for entry in BUILTINS {
            let wf: Workflow = serde_saphyr::from_str(entry.content).unwrap_or_else(|err| {
                panic!(
                    "Invalid builtin workflow '{}' fixture: {err}",
                    entry.filename
                )
            });
            assert_eq!(
                entry.description, wf.description,
                "BuiltinEntry.description for '{}' does not match YAML; update BUILTINS metadata when changing YAML description",
                entry.filename
            );
        }
    }

    #[test]
    fn get_builtin_facet_returns_content() {
        let content = get_builtin_facet(FacetKind::Policy, "coding");
        assert!(content.is_some());
        assert!(!content.unwrap().is_empty());
    }

    #[test]
    fn get_builtin_facet_returns_none_for_unknown() {
        assert!(get_builtin_facet(FacetKind::Policy, "nonexistent").is_none());
    }

    #[test]
    fn list_builtin_facet_keys_filters_by_kind() {
        let policies = list_builtin_facet_keys(FacetKind::Policy);
        assert_eq!(policies.len(), 4);
        assert!(policies.contains(&"coding"));
        assert!(policies.contains(&"review"));
        assert!(policies.contains(&"planning"));
        assert!(policies.contains(&"plan-review"));

        let instructions = list_builtin_facet_keys(FacetKind::Instruction);
        assert_eq!(instructions.len(), 20);

        let output_contracts = list_builtin_facet_keys(FacetKind::OutputContract);
        assert_eq!(output_contracts.len(), 3);
    }

    #[test]
    fn is_builtin_workflow_works() {
        assert!(is_builtin_workflow("spec-driven-development"));
        assert!(!is_builtin_workflow("custom-workflow"));
    }

    #[test]
    fn is_builtin_facet_works() {
        assert!(is_builtin_facet(FacetKind::Policy, "coding"));
        assert!(!is_builtin_facet(FacetKind::Policy, "custom"));
    }

    /// Gherkin: ビルトインファセット定義に persona 系の定義が存在しない
    /// `BUILTIN_FACETS` 配列に含まれる種別が4種（policy/knowledge/instruction/output_contract）に
    /// 限定されることを確認する。`FacetKind::Persona` enum variant は廃止済みのため、ここでは
    /// 「4種以外の種別が含まれない」ことを網羅的に検証する。
    #[test]
    fn builtin_facets_contains_only_four_kinds_no_persona() {
        let total: usize = [
            FacetKind::Policy,
            FacetKind::Knowledge,
            FacetKind::Instruction,
            FacetKind::OutputContract,
        ]
        .iter()
        .map(|k| list_builtin_facet_keys(*k).len())
        .sum();
        assert_eq!(
            total,
            BUILTIN_FACETS.len(),
            "BUILTIN_FACETS must only contain the four kinds (policy/knowledge/instruction/output_contract); \
             any entry not covered by these kinds (e.g. a persona kind) would break this invariant"
        );
    }

    /// Gherkin: ビルトインファセット定義に persona 系の定義が存在しない
    /// `src-tauri/src/workflow/builtin_facets/` 配下に `personas/` ディレクトリが存在しないこと、
    /// また `BUILTIN_FACETS` のキー一覧に persona 系のキーが混ざっていないことを確認する。
    #[test]
    fn builtin_facets_directory_has_no_personas_subdir() {
        let builtin_facets_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/workflow/builtin_facets");
        assert!(
            builtin_facets_dir.exists(),
            "builtin_facets dir must exist: {}",
            builtin_facets_dir.display()
        );
        let entries: Vec<String> = std::fs::read_dir(&builtin_facets_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !entries.iter().any(|name| name == "personas"),
            "builtin_facets/ must not contain a 'personas' subdirectory, found entries: {entries:?}"
        );
        // 念のため、4 種のサブディレクトリ以外を許容しない（将来の persona 復活を即座に検出）
        for name in &entries {
            assert!(
                matches!(
                    name.as_str(),
                    "policies" | "knowledge" | "instructions" | "output_contracts"
                ),
                "unexpected builtin_facets/ entry: {name}"
            );
        }
    }

    #[test]
    fn spec_driven_development_routes_plan_review_needs_fix_to_policy_approval() {
        let wf = load_builtin_workflow_resolved("spec-driven-development")
            .expect("builtin load must succeed")
            .expect("builtin must exist for known name");
        let review = wf
            .nodes
            .iter()
            .find(|n| n.name == "plan_review_parallel")
            .unwrap();
        let aggregate = review.aggregate.as_ref().unwrap();
        assert_eq!(aggregate.all_match.as_deref(), Some("LGTM"));
        assert_eq!(aggregate.then, "plan_approval");
        assert_eq!(aggregate.r#else, "plan_fix_policy");

        let policy = wf
            .nodes
            .iter()
            .find(|n| n.name == "plan_fix_policy")
            .unwrap();
        assert_eq!(
            policy.output_contract.as_deref(),
            Some("approved-fix-policy")
        );
        let policy_inputs = policy.pass_output_from.as_ref().unwrap();
        assert!(policy_inputs.contains(&"plan_review_parallel".to_string()));
        assert!(policy_inputs.contains(&"plan_requirements".to_string()));
    }

    /// [02] schema 境界: 組み込み workflow が新 schema として等価実行可能であることの
    /// 構造スナップショット相当テスト。step 数・全 node の node_type・並列構成・aggregate 条件・
    /// approval rules・cycle guard / reset・主要遷移を網羅的に検証する。
    #[test]
    fn spec_driven_development_structural_snapshot() {
        use crate::workflow::schema::NodeType;
        let wf = load_builtin_workflow_resolved("spec-driven-development")
            .expect("builtin load must succeed")
            .expect("builtin must exist for known name");

        // 全 node の (name, node_type) を構造スナップショットとして固定する
        let observed: Vec<(String, NodeType)> = wf
            .nodes
            .iter()
            .map(|n| (n.name.clone(), n.node_type))
            .collect();
        // 全 node 名と種別の組（旧表現と等価な実行構造を担保）。
        // 追加・削除・種別変更は本テストを更新して明示的に承認すること。
        let expected: Vec<(&str, NodeType)> = vec![
            ("plan_requirements", NodeType::Approval),
            ("plan_behavior", NodeType::Approval),
            ("plan_architecture", NodeType::Approval),
            ("plan_review_parallel", NodeType::Parallel),
            ("plan_fix_policy", NodeType::Approval),
            ("plan_fix", NodeType::Agent),
            ("plan_approval", NodeType::Approval),
            ("implement", NodeType::Agent),
            ("code_review_parallel", NodeType::Parallel),
            ("implementation_fix_policy", NodeType::Approval),
            ("fix", NodeType::Agent),
            ("implementation_approval", NodeType::Approval),
        ];
        let expected: Vec<(String, NodeType)> = expected
            .into_iter()
            .map(|(n, t)| (n.to_string(), t))
            .collect();
        assert_eq!(observed, expected, "spec-driven-development の構造は固定");

        // 並列 node の子構成（名前 + 種別を具体値で固定）
        let plan_review = wf
            .nodes
            .iter()
            .find(|n| n.name == "plan_review_parallel")
            .unwrap();
        let plan_children: Vec<(String, NodeType)> = plan_review
            .parallel_children
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| (c.name.clone(), c.node_type))
            .collect();
        let expected_plan_children: Vec<(String, NodeType)> = vec![
            ("plan_review_completeness", NodeType::Agent),
            ("plan_review_clarity", NodeType::Agent),
            ("plan_review_security", NodeType::Agent),
            ("plan_review_consistency", NodeType::Agent),
        ]
        .into_iter()
        .map(|(n, t)| (n.to_string(), t))
        .collect();
        assert_eq!(plan_children, expected_plan_children);

        let code_review = wf
            .nodes
            .iter()
            .find(|n| n.name == "code_review_parallel")
            .unwrap();
        let code_children: Vec<(String, NodeType)> = code_review
            .parallel_children
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| (c.name.clone(), c.node_type))
            .collect();
        let expected_code_children: Vec<(String, NodeType)> = vec![
            ("code_review_acceptance", NodeType::Agent),
            ("code_review_structure", NodeType::Agent),
            ("code_review_quality", NodeType::Agent),
            ("code_review_test", NodeType::Agent),
            ("code_review_security", NodeType::Agent),
            ("code_review_architecture", NodeType::Agent),
        ]
        .into_iter()
        .map(|(n, t)| (n.to_string(), t))
        .collect();
        assert_eq!(code_children, expected_code_children);

        // aggregate 条件と遷移先
        let plan_agg = plan_review.aggregate.as_ref().unwrap();
        assert_eq!(plan_agg.all_match.as_deref(), Some("LGTM"));
        assert!(plan_agg.any_match.is_none());
        assert_eq!(plan_agg.then, "plan_approval");
        assert_eq!(plan_agg.r#else, "plan_fix_policy");

        let code_agg = code_review.aggregate.as_ref().unwrap();
        assert_eq!(code_agg.all_match.as_deref(), Some("LGTM"));
        assert!(code_agg.any_match.is_none());
        assert_eq!(code_agg.then, "implementation_approval");
        assert_eq!(code_agg.r#else, "implementation_fix_policy");

        // approval rules: 各 approval の reject 遷移先を具体値で固定
        let plan_approval = wf.nodes.iter().find(|n| n.name == "plan_approval").unwrap();
        assert_eq!(plan_approval.transition_rules.len(), 1);
        assert_eq!(plan_approval.transition_rules[0].r#match, "reject");
        assert_eq!(plan_approval.transition_rules[0].next, "plan_fix_policy");

        let impl_approval = wf
            .nodes
            .iter()
            .find(|n| n.name == "implementation_approval")
            .unwrap();
        assert_eq!(impl_approval.transition_rules.len(), 1);
        assert_eq!(impl_approval.transition_rules[0].r#match, "reject");
        assert_eq!(
            impl_approval.transition_rules[0].next,
            "implementation_fix_policy"
        );

        // cycle guard の具体値と reset 対象を固定
        let plan_fix = wf.nodes.iter().find(|n| n.name == "plan_fix").unwrap();
        let plan_fix_guard = plan_fix.cycle_guard.as_ref().unwrap();
        assert_eq!(plan_fix_guard.max_iterations, 2);
        assert_eq!(
            plan_fix_guard.on_exhausted.as_deref(),
            Some("plan_approval")
        );
        assert_eq!(
            plan_approval.resets_cycle_for.as_deref(),
            Some(&["plan_fix".to_string()][..])
        );

        let fix = wf.nodes.iter().find(|n| n.name == "fix").unwrap();
        let fix_guard = fix.cycle_guard.as_ref().unwrap();
        assert_eq!(fix_guard.max_iterations, 3);
        assert_eq!(
            fix_guard.on_exhausted.as_deref(),
            Some("implementation_approval")
        );
        assert_eq!(
            impl_approval.resets_cycle_for.as_deref(),
            Some(&["fix".to_string()][..])
        );

        // 主要 transition / pass_output_from を固定
        // plan_fix: NO_FIX_NEEDED → plan_approval / ".*" → plan_review_parallel
        assert_eq!(plan_fix.transition_rules.len(), 2);
        assert_eq!(plan_fix.transition_rules[0].r#match, "NO_FIX_NEEDED");
        assert_eq!(plan_fix.transition_rules[0].next, "plan_approval");
        assert_eq!(plan_fix.transition_rules[1].r#match, ".*");
        assert_eq!(plan_fix.transition_rules[1].next, "plan_review_parallel");
        // fix: NO_FIX_NEEDED → implementation_approval / ".*" → code_review_parallel
        assert_eq!(fix.transition_rules.len(), 2);
        assert_eq!(fix.transition_rules[0].r#match, "NO_FIX_NEEDED");
        assert_eq!(fix.transition_rules[0].next, "implementation_approval");
        assert_eq!(fix.transition_rules[1].r#match, ".*");
        assert_eq!(fix.transition_rules[1].next, "code_review_parallel");

        // built-in は新 schema validation を通過する
        crate::workflow::validation::validate(&wf).expect("built-in は新 schema として有効");
    }

    /// [02] schema 境界: built-in workflow の load 経路で `resolved_facets` が populated
    /// されることを担保する。top-level node と parallel child の両方で、policy/knowledge/
    /// instruction/output_contract のいずれかが指定されていれば本文が解決済みであること
    /// を検証する。これにより、共通 loader が built-in 経路で削られても CI で検知される。
    #[test]
    fn spec_driven_development_resolves_facets_on_load() {
        let wf = load_builtin_workflow_resolved("spec-driven-development")
            .expect("builtin load must succeed")
            .expect("builtin must exist for known name");

        let mut top_resolved_count = 0;
        for node in &wf.nodes {
            if node.policy.is_some() {
                assert!(
                    node.resolved_facets.policy.is_some(),
                    "node '{}' has policy ref but resolved_facets.policy is None",
                    node.name
                );
                top_resolved_count += 1;
            }
            if node.knowledge.is_some() {
                assert!(
                    node.resolved_facets.knowledge.is_some(),
                    "node '{}' has knowledge ref but resolved_facets.knowledge is None",
                    node.name
                );
                top_resolved_count += 1;
            }
            if node.instruction.is_some() {
                assert!(
                    node.resolved_facets.instruction.is_some(),
                    "node '{}' has instruction ref but resolved_facets.instruction is None",
                    node.name
                );
                top_resolved_count += 1;
            }
            if node.output_contract.is_some() {
                assert!(
                    node.resolved_facets.output_contract.is_some(),
                    "node '{}' has output_contract ref but resolved_facets.output_contract is None",
                    node.name
                );
                top_resolved_count += 1;
            }
        }
        assert!(
            top_resolved_count > 0,
            "spec-driven-development must populate resolved_facets on at least one top-level node"
        );

        // parallel child でも同様に解決済みであることを検証する
        let mut child_resolved_count = 0;
        for node in &wf.nodes {
            let Some(children) = node.parallel_children.as_ref() else {
                continue;
            };
            for child in children {
                if child.policy.is_some() {
                    assert!(
                        child.resolved_facets.policy.is_some(),
                        "child '{}/{}' has policy ref but resolved_facets.policy is None",
                        node.name,
                        child.name
                    );
                    child_resolved_count += 1;
                }
                if child.knowledge.is_some() {
                    assert!(
                        child.resolved_facets.knowledge.is_some(),
                        "child '{}/{}' has knowledge ref but resolved_facets.knowledge is None",
                        node.name,
                        child.name
                    );
                    child_resolved_count += 1;
                }
                if child.instruction.is_some() {
                    assert!(
                        child.resolved_facets.instruction.is_some(),
                        "child '{}/{}' has instruction ref but resolved_facets.instruction is None",
                        node.name,
                        child.name
                    );
                    child_resolved_count += 1;
                }
                if child.output_contract.is_some() {
                    assert!(
                        child.resolved_facets.output_contract.is_some(),
                        "child '{}/{}' has output_contract ref but resolved_facets.output_contract is None",
                        node.name,
                        child.name
                    );
                    child_resolved_count += 1;
                }
            }
        }
        assert!(
            child_resolved_count > 0,
            "spec-driven-development must populate resolved_facets on at least one parallel child"
        );
    }

    #[test]
    fn spec_driven_development_passes_approved_policy_to_fix_steps() {
        let wf = load_builtin_workflow_resolved("spec-driven-development")
            .expect("builtin load must succeed")
            .expect("builtin must exist for known name");
        let plan_fix = wf.nodes.iter().find(|n| n.name == "plan_fix").unwrap();
        assert_eq!(plan_fix.pass_previous_response, None);
        let plan_fix_inputs = plan_fix.pass_output_from.as_ref().unwrap();
        assert!(plan_fix_inputs.contains(&"plan_requirements".to_string()));
        assert!(plan_fix_inputs.contains(&"plan_fix_policy".to_string()));

        let code_review = wf
            .nodes
            .iter()
            .find(|n| n.name == "code_review_parallel")
            .unwrap();
        let aggregate = code_review.aggregate.as_ref().unwrap();
        assert_eq!(aggregate.then, "implementation_approval");
        assert_eq!(aggregate.r#else, "implementation_fix_policy");
        let implementation_policy = wf
            .nodes
            .iter()
            .find(|n| n.name == "implementation_fix_policy")
            .unwrap();
        assert_eq!(
            implementation_policy.output_contract.as_deref(),
            Some("approved-fix-policy")
        );
        let implementation_policy_inputs = implementation_policy.pass_output_from.as_ref().unwrap();
        assert!(implementation_policy_inputs.contains(&"code_review_parallel".to_string()));
        assert!(implementation_policy_inputs.contains(&"plan_requirements".to_string()));

        let fix = wf.nodes.iter().find(|n| n.name == "fix").unwrap();
        assert_eq!(fix.pass_previous_response, None);
        let fix_inputs = fix.pass_output_from.as_ref().unwrap();
        assert!(fix_inputs.contains(&"implementation_fix_policy".to_string()));
        assert!(fix_inputs.contains(&"plan_requirements".to_string()));
    }
}
