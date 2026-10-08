use crate::domain::local_event::identifiers::{Revision, StreamId};
use crate::domain::local_event::record::SessionProjectionRecord;

/// Revision guard for CAS rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionGuard {
    /// The row must not exist yet.
    Absent,
    /// The row must exist at exactly this revision.
    Expected(Revision),
}

/// Complete bounded session / queue / lifecycle read-model row.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionProjectionMutation {
    pub session_id: String,
    pub projection: SessionProjectionRecord,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Removes every durable AgentSession payload except the newly
/// appended tombstone. A released provider ownership aggregate is removed in
/// the same transaction so the provider session can be claimed again without
/// retaining its resume identifier in Releash state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionRemovalMutation {
    pub node_event_tree_id: String,
    pub ownership_projection_id: Option<String>,
    pub ownership_stream: Option<StreamId>,
    pub ownership_expected: Option<Revision>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalStateMutation {
    SessionProjection(SessionProjectionMutation),
    AgentSessionRemoval(AgentSessionRemovalMutation),
}

impl LocalStateMutation {
    /// Stable, explicitly-versioned semantic bytes for commit idempotency.
    ///
    /// Generic projection commits currently accept only these mutation
    /// families. Keeping the match closed makes a newly-added family fail
    /// admission until its identity encoding is deliberately specified; Rust
    /// `Debug` output is never a persistent identity contract.
    pub fn canonical_identity_v1(&self) -> Result<Vec<u8>, &'static str> {
        fn field(bytes: &mut Vec<u8>, value: &[u8]) {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value);
        }
        fn text(bytes: &mut Vec<u8>, value: &str) {
            field(bytes, value.as_bytes());
        }
        fn revision(bytes: &mut Vec<u8>, value: Revision) {
            bytes.extend_from_slice(&value.value().to_be_bytes());
        }
        let mut bytes = b"local_state_mutation_identity_v1".to_vec();
        match self {
            // Projection identity is the canonical Stored*V1 representation,
            // which belongs to the persistence gateway. Calling this
            // domain-only encoder for a projection would silently change
            // existing replay identities, so projection-capable commit paths
            // must use the gateway canonicalizer.
            Self::SessionProjection(_) => {
                return Err("projection identity-v1 encoding is gateway-owned")
            }
            Self::AgentSessionRemoval(m) => {
                text(&mut bytes, "agent_session_removal");
                text(&mut bytes, &m.node_event_tree_id);
                match (
                    &m.ownership_projection_id,
                    &m.ownership_stream,
                    m.ownership_expected,
                ) {
                    (Some(projection_id), Some(stream), Some(expected)) => {
                        bytes.push(1);
                        text(&mut bytes, projection_id);
                        text(&mut bytes, stream.as_str());
                        revision(&mut bytes, expected);
                    }
                    (None, None, None) => bytes.push(0),
                    _ => return Err("incomplete provider ownership removal"),
                }
            }
        }
        Ok(bytes)
    }

    pub fn approximate_bytes(&self) -> usize {
        match self {
            Self::SessionProjection(m) => m.projection.semantic_bytes().saturating_add(64),
            Self::AgentSessionRemoval(m) => {
                m.node_event_tree_id.len()
                    + m.ownership_projection_id.as_ref().map_or(0, String::len)
                    + m.ownership_stream
                        .as_ref()
                        .map_or(0, |stream| stream.as_str().len())
                    + 96
            }
        }
    }
}
