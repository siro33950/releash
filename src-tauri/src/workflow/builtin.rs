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
        key: "quality-check",
        content: include_str!("builtin_facets/instructions/quality-check.md"),
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
        key: "fix-result",
        content: include_str!("builtin_facets/output_contracts/fix-result.md"),
    },
    BuiltinFacetEntry {
        kind: FacetKind::OutputContract,
        key: "spec-file-path",
        content: include_str!("builtin_facets/output_contracts/spec-file-path.md"),
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
        assert_eq!(instructions.len(), 19);

        let output_contracts = list_builtin_facet_keys(FacetKind::OutputContract);
        assert_eq!(output_contracts.len(), 3);

        let personas = list_builtin_facet_keys(FacetKind::Persona);
        assert!(personas.is_empty());
    }

    #[test]
    fn is_builtin_facet_works() {
        assert!(is_builtin_facet(FacetKind::Policy, "coding"));
        assert!(!is_builtin_facet(FacetKind::Policy, "custom"));
    }
}
