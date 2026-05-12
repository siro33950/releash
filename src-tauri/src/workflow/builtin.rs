use super::facet::FacetKind;
use super::schema::{Summary, Workflow};

const BUILTIN_SPEC_DRIVEN_DEVELOPMENT: &str = include_str!("builtin/spec-driven-development.yml");

struct BuiltinEntry {
    filename: &'static str,
    content: &'static str,
}

const BUILTINS: &[BuiltinEntry] = &[BuiltinEntry {
    filename: "spec-driven-development.yml",
    content: BUILTIN_SPEC_DRIVEN_DEVELOPMENT,
}];

pub fn get_builtin_workflow(name: &str) -> Option<Workflow> {
    BUILTINS
        .iter()
        .find(|e| e.filename.strip_suffix(".yml") == Some(name))
        .map(|e| {
            serde_saphyr::from_str(e.content)
                .unwrap_or_else(|err| panic!("Invalid builtin workflow '{}': {err}", e.filename))
        })
}

pub fn list_builtin_workflows() -> Vec<Summary> {
    BUILTINS
        .iter()
        .map(|e| {
            let wf: Workflow = serde_saphyr::from_str(e.content)
                .unwrap_or_else(|err| panic!("Invalid builtin workflow '{}': {err}", e.filename));
            Summary {
                name: e
                    .filename
                    .strip_suffix(".yml")
                    .unwrap_or(e.filename)
                    .to_string(),
                description: wf.description,
                builtin: true,
                is_running: false,
            }
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
    fn get_builtin_workflow_returns_valid_workflow() {
        let wf = get_builtin_workflow("spec-driven-development").unwrap();
        assert_eq!(wf.name, "spec-driven-development");
        assert!(wf.builtin);
    }

    #[test]
    fn get_builtin_workflow_returns_none_for_unknown() {
        assert!(get_builtin_workflow("nonexistent").is_none());
    }

    #[test]
    fn list_builtin_workflows_returns_all() {
        let summaries = list_builtin_workflows();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "spec-driven-development");
        assert!(summaries[0].builtin);
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
        let wf = get_builtin_workflow("spec-driven-development").unwrap();
        let review = wf
            .steps
            .iter()
            .find(|step| step.name == "plan_review_parallel")
            .unwrap();
        let aggregate = review.aggregate.as_ref().unwrap();
        assert_eq!(aggregate.all_match.as_deref(), Some("LGTM"));
        assert_eq!(aggregate.then, "plan_approval");
        assert_eq!(aggregate.r#else, "plan_fix_policy");

        let policy = wf
            .steps
            .iter()
            .find(|step| step.name == "plan_fix_policy")
            .unwrap();
        assert_eq!(
            policy.output_contract.as_deref(),
            Some("approved-fix-policy")
        );
        assert!(policy
            .pass_output_from
            .as_ref()
            .unwrap()
            .contains(&"plan_review_parallel".to_string()));
        assert!(policy
            .pass_output_from
            .as_ref()
            .unwrap()
            .contains(&"plan_requirements".to_string()));
    }

    #[test]
    fn spec_driven_development_passes_approved_policy_to_fix_steps() {
        let wf = get_builtin_workflow("spec-driven-development").unwrap();
        let plan_fix = wf
            .steps
            .iter()
            .find(|step| step.name == "plan_fix")
            .unwrap();
        assert_eq!(plan_fix.pass_previous_response, None);
        let plan_fix_inputs = plan_fix.pass_output_from.as_ref().unwrap();
        assert!(plan_fix_inputs.contains(&"plan_requirements".to_string()));
        assert!(plan_fix_inputs.contains(&"plan_fix_policy".to_string()));

        let code_review = wf
            .steps
            .iter()
            .find(|step| step.name == "code_review_parallel")
            .unwrap();
        let aggregate = code_review.aggregate.as_ref().unwrap();
        assert_eq!(aggregate.then, "implementation_approval");
        assert_eq!(aggregate.r#else, "implementation_fix_policy");
        let implementation_policy = wf
            .steps
            .iter()
            .find(|step| step.name == "implementation_fix_policy")
            .unwrap();
        assert_eq!(
            implementation_policy.output_contract.as_deref(),
            Some("approved-fix-policy")
        );
        let implementation_policy_inputs = implementation_policy.pass_output_from.as_ref().unwrap();
        assert!(implementation_policy_inputs.contains(&"code_review_parallel".to_string()));
        assert!(implementation_policy_inputs.contains(&"plan_requirements".to_string()));

        let fix = wf.steps.iter().find(|step| step.name == "fix").unwrap();
        assert_eq!(fix.pass_previous_response, None);
        let fix_inputs = fix.pass_output_from.as_ref().unwrap();
        assert!(fix_inputs.contains(&"implementation_fix_policy".to_string()));
        assert!(fix_inputs.contains(&"plan_requirements".to_string()));
    }
}
