use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::domain::agent_session::value_objects::{
    ContextEpoch, ContextEpochId, ContextEpochIdentity, ContextRevision, ContextSnapshot,
    ContextSourceKind, InstructionOrigin, ReplacementAction, ReplacementTrigger,
    ResolvedInstruction,
};

pub(crate) fn replacement_action(
    trigger: ReplacementTrigger,
    kind: ContextSourceKind,
) -> ReplacementAction {
    match trigger {
        ReplacementTrigger::None => ReplacementAction::Retain,
        ReplacementTrigger::BackendChanged | ReplacementTrigger::ModelChanged => match kind {
            ContextSourceKind::ProjectInstructions | ContextSourceKind::BackendModelIdentity => {
                ReplacementAction::Discard
            }
            _ => ReplacementAction::Retain,
        },
        ReplacementTrigger::WorktreeChanged => match kind {
            ContextSourceKind::RepoSummary
            | ContextSourceKind::DiffReviewSnapshot
            | ContextSourceKind::OpenEditorSelection
            | ContextSourceKind::Mentions
            | ContextSourceKind::ProjectInstructions => ReplacementAction::Rebuild,
            _ => ReplacementAction::Retain,
        },
        ReplacementTrigger::InstructionFileChanged => match kind {
            ContextSourceKind::ProjectInstructions => ReplacementAction::Rebuild,
            _ => ReplacementAction::Retain,
        },
    }
}

pub(crate) fn next_epoch_for_identity(
    current: Option<&ContextEpoch>,
    identity: ContextEpochIdentity,
) -> (ContextEpoch, ReplacementTrigger) {
    let Some(current) = current else {
        return (
            ContextEpoch {
                id: ContextEpochId::default(),
                backend_id: identity.backend_id,
                model_id: identity.model_id,
                worktree_path: identity.worktree_path,
            },
            ReplacementTrigger::None,
        );
    };
    if current.identity_matches(&identity) {
        return (current.clone(), ReplacementTrigger::None);
    }
    let trigger = if current.worktree_path != identity.worktree_path {
        ReplacementTrigger::WorktreeChanged
    } else if current.backend_id != identity.backend_id {
        ReplacementTrigger::BackendChanged
    } else {
        ReplacementTrigger::ModelChanged
    };
    (
        ContextEpoch {
            id: current.id.next(),
            backend_id: identity.backend_id,
            model_id: identity.model_id,
            worktree_path: identity.worktree_path,
        },
        trigger,
    )
}

pub(crate) fn snapshot_is_stale(
    snapshot: &ContextSnapshot,
    current_epoch_id: ContextEpochId,
    latest_revision_for_kind: Option<ContextRevision>,
) -> bool {
    snapshot.epoch_id != current_epoch_id
        || latest_revision_for_kind.is_some_and(|revision| snapshot.revision < revision)
}

pub(crate) fn dedup_instructions(
    instructions: Vec<ResolvedInstruction>,
) -> Vec<ResolvedInstruction> {
    let mut sorted = instructions;
    sorted.sort_by(|a, b| {
        a.scope_depth
            .cmp(&b.scope_depth)
            .then_with(|| {
                instruction_origin_order(a.origin).cmp(&instruction_origin_order(b.origin))
            })
            .then_with(|| normalized_instruction_path(a).cmp(&normalized_instruction_path(b)))
    });

    let mut seen_paths = HashSet::new();
    let mut seen_fingerprints = HashSet::new();
    let mut deduped = Vec::new();
    for instruction in sorted {
        let normalized_path = normalized_instruction_path(&instruction);
        let duplicate_path = normalized_path
            .as_ref()
            .is_some_and(|path| seen_paths.contains(path));
        let duplicate_content = seen_fingerprints.contains(&instruction.fingerprint);
        if duplicate_path || duplicate_content {
            continue;
        }
        if let Some(path) = normalized_path {
            seen_paths.insert(path);
        }
        seen_fingerprints.insert(instruction.fingerprint.clone());
        deduped.push(instruction);
    }
    deduped
}

pub(crate) fn latest_revisions_by_kind(
    snapshots: &[ContextSnapshot],
) -> HashMap<ContextSourceKind, ContextRevision> {
    let mut revisions = HashMap::new();
    for snapshot in snapshots {
        revisions
            .entry(snapshot.kind)
            .and_modify(|revision: &mut ContextRevision| {
                if *revision < snapshot.revision {
                    *revision = snapshot.revision;
                }
            })
            .or_insert(snapshot.revision);
    }
    revisions
}

fn instruction_origin_order(origin: InstructionOrigin) -> usize {
    match origin {
        InstructionOrigin::RepoHierarchy => 0,
        InstructionOrigin::FileNeighbor => 1,
        InstructionOrigin::WorkflowFacet => 2,
    }
}

fn normalized_instruction_path(instruction: &ResolvedInstruction) -> Option<PathBuf> {
    instruction
        .source_path
        .as_ref()
        .map(|path| normalize_path_components(path))
}

pub(crate) fn normalize_path_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_matrix_matches_design() {
        assert_eq!(
            replacement_action(
                ReplacementTrigger::BackendChanged,
                ContextSourceKind::ProjectInstructions
            ),
            ReplacementAction::Discard
        );
        assert_eq!(
            replacement_action(
                ReplacementTrigger::ModelChanged,
                ContextSourceKind::BackendModelIdentity
            ),
            ReplacementAction::Discard
        );
        assert_eq!(
            replacement_action(
                ReplacementTrigger::WorktreeChanged,
                ContextSourceKind::RepoSummary
            ),
            ReplacementAction::Rebuild
        );
        assert_eq!(
            replacement_action(
                ReplacementTrigger::WorktreeChanged,
                ContextSourceKind::Mentions
            ),
            ReplacementAction::Rebuild
        );
        assert_eq!(
            replacement_action(
                ReplacementTrigger::InstructionFileChanged,
                ContextSourceKind::ProjectInstructions
            ),
            ReplacementAction::Rebuild
        );
        for kind in ContextSourceKind::ALL {
            assert_eq!(
                replacement_action(ReplacementTrigger::None, kind),
                ReplacementAction::Retain
            );
        }
        assert_eq!(
            replacement_action(
                ReplacementTrigger::InstructionFileChanged,
                ContextSourceKind::RepoSummary
            ),
            ReplacementAction::Retain
        );
    }

    #[test]
    fn epoch_identity_change_selects_replacement_trigger() {
        let current = ContextEpoch {
            id: ContextEpochId(7),
            backend_id: Some("claude".to_string()),
            model_id: Some("sonnet".to_string()),
            worktree_path: "/repo".to_string(),
        };

        let (epoch, trigger) = next_epoch_for_identity(
            Some(&current),
            ContextEpochIdentity {
                backend_id: Some("codex".to_string()),
                model_id: Some("gpt-5".to_string()),
                worktree_path: "/repo".to_string(),
            },
        );

        assert_eq!(epoch.id, ContextEpochId(8));
        assert_eq!(trigger, ReplacementTrigger::BackendChanged);

        let (epoch, trigger) = next_epoch_for_identity(
            Some(&current),
            ContextEpochIdentity {
                backend_id: Some("claude".to_string()),
                model_id: Some("haiku".to_string()),
                worktree_path: "/repo".to_string(),
            },
        );
        assert_eq!(epoch.id, ContextEpochId(8));
        assert_eq!(trigger, ReplacementTrigger::ModelChanged);

        let (epoch, trigger) = next_epoch_for_identity(
            Some(&current),
            ContextEpochIdentity {
                backend_id: Some("claude".to_string()),
                model_id: Some("sonnet".to_string()),
                worktree_path: "/other".to_string(),
            },
        );
        assert_eq!(epoch.id, ContextEpochId(8));
        assert_eq!(trigger, ReplacementTrigger::WorktreeChanged);
    }

    #[test]
    fn stale_detection_uses_epoch_and_latest_revision() {
        let snapshot = ContextSnapshot {
            kind: ContextSourceKind::RepoSummary,
            epoch_id: ContextEpochId(2),
            revision: ContextRevision(3),
            fingerprint: "a".to_string(),
            payload: "repo".to_string(),
        };

        assert!(snapshot_is_stale(
            &snapshot,
            ContextEpochId(1),
            Some(ContextRevision(3))
        ));
        assert!(snapshot_is_stale(
            &snapshot,
            ContextEpochId(2),
            Some(ContextRevision(4))
        ));
        assert!(!snapshot_is_stale(
            &snapshot,
            ContextEpochId(2),
            Some(ContextRevision(3))
        ));
    }

    #[test]
    fn instructions_are_deduped_by_path_or_content_and_ordered_by_scope() {
        let root = ResolvedInstruction::new(
            InstructionOrigin::RepoHierarchy,
            Some(PathBuf::from("/repo/AGENTS.md")),
            "root",
            "fp-root",
            0,
        );
        let same_path = ResolvedInstruction::new(
            InstructionOrigin::FileNeighbor,
            Some(PathBuf::from("/repo/./AGENTS.md")),
            "root changed but same file",
            "fp-root-changed",
            1,
        );
        let local = ResolvedInstruction::new(
            InstructionOrigin::FileNeighbor,
            Some(PathBuf::from("/repo/src/AGENTS.md")),
            "local",
            "fp-local",
            2,
        );
        let same_content = ResolvedInstruction::new(
            InstructionOrigin::WorkflowFacet,
            None,
            "local",
            "fp-local",
            3,
        );

        let deduped = dedup_instructions(vec![same_content, local, same_path, root]);

        assert_eq!(
            deduped
                .iter()
                .map(|instruction| instruction.content.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "local"]
        );
    }
}
