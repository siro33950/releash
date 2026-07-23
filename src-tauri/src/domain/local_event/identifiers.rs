//! Store identities for the permanent local event store.
//!
//! These newtypes carry the invariants from the issues-1499 design "Store
//! identities" section. They deliberately have no serde, rusqlite, or
//! filesystem dependency; adapters convert at their own boundary.

#![allow(dead_code)] // Identity invariants expose the complete persisted-store vocabulary.

use std::fmt;

pub const MAX_IDENTITY_BYTES: usize = 128;

const AGENT_SESSION_PREFIX: &str = "agent-session:";
const WORKFLOW_PREFIX: &str = "workflow:";
const APPLICATION_STREAM: &str = "application";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    Empty,
    TooLong { max: usize },
    InvalidCharacter { found: char },
    InvalidStreamNamespace,
    OutOfRange { value: i64 },
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "identity must not be empty"),
            Self::TooLong { max } => write!(f, "identity exceeds {max} bytes"),
            Self::InvalidCharacter { found } => {
                write!(f, "identity contains invalid character {found:?}")
            }
            Self::InvalidStreamNamespace => write!(
                f,
                "stream id must be agent-session:<id>, workflow:<id>, or application"
            ),
            Self::OutOfRange { value } => write!(f, "sequence value {value} is out of range"),
        }
    }
}

impl std::error::Error for IdentityError {}

fn validate_identity_text(raw: &str) -> Result<(), IdentityError> {
    if raw.is_empty() {
        return Err(IdentityError::Empty);
    }
    if raw.len() > MAX_IDENTITY_BYTES {
        return Err(IdentityError::TooLong {
            max: MAX_IDENTITY_BYTES,
        });
    }
    for ch in raw.chars() {
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-')) {
            return Err(IdentityError::InvalidCharacter { found: ch });
        }
    }
    Ok(())
}

/// Namespace classification of a [`StreamId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamKind {
    AgentSession { session_id: String },
    Workflow { execution_id: String },
    Application,
}

/// Namespaced opaque stream identity: `agent-session:<id>`, `workflow:<id>`,
/// or the singleton `application` stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamId(String);

impl StreamId {
    pub fn parse(raw: &str) -> Result<Self, IdentityError> {
        if raw != APPLICATION_STREAM {
            let suffix = raw
                .strip_prefix(AGENT_SESSION_PREFIX)
                .or_else(|| raw.strip_prefix(WORKFLOW_PREFIX))
                .ok_or(IdentityError::InvalidStreamNamespace)?;
            if suffix.is_empty() {
                return Err(IdentityError::InvalidStreamNamespace);
            }
        }
        validate_identity_text(raw)?;
        Ok(Self(raw.to_string()))
    }

    pub fn agent_session(session_id: &str) -> Result<Self, IdentityError> {
        Self::parse(&format!("{AGENT_SESSION_PREFIX}{session_id}"))
    }

    pub fn workflow(execution_id: &str) -> Result<Self, IdentityError> {
        Self::parse(&format!("{WORKFLOW_PREFIX}{execution_id}"))
    }

    pub fn application() -> Self {
        Self(APPLICATION_STREAM.to_string())
    }

    pub fn kind(&self) -> StreamKind {
        if let Some(id) = self.0.strip_prefix(AGENT_SESSION_PREFIX) {
            StreamKind::AgentSession {
                session_id: id.to_string(),
            }
        } else if let Some(id) = self.0.strip_prefix(WORKFLOW_PREFIX) {
            StreamKind::Workflow {
                execution_id: id.to_string(),
            }
        } else {
            StreamKind::Application
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! text_identity {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn parse(raw: &str) -> Result<Self, IdentityError> {
                validate_identity_text(raw)?;
                Ok(Self(raw.to_string()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

text_identity!(
    /// Immutable identity of one stored event.
    EventId
);
text_identity!(
    /// Identity of one atomic commit; retries and `resolve_commit` reuse it.
    CommitIdentity
);

macro_rules! sequence_identity {
    ($(#[$doc:meta])* $name:ident, $min:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(i64);

        impl $name {
            pub const MIN: i64 = $min;

            pub fn new(value: i64) -> Result<Self, IdentityError> {
                if value < $min {
                    return Err(IdentityError::OutOfRange { value });
                }
                Ok(Self(value))
            }

            pub fn value(self) -> i64 {
                self.0
            }

            /// Next value, or `None` at the signed 64-bit boundary so the
            /// caller can fail with a typed capacity error before overflow.
            pub fn next(self) -> Option<Self> {
                self.0.checked_add(1).map(Self)
            }
        }
    };
}

sequence_identity!(
    /// Per-stream head version. `0` means the stream has no events yet.
    StreamVersion,
    0
);
sequence_identity!(
    /// Total order over every committed event in the store.
    GlobalSequence,
    1
);
sequence_identity!(
    /// Order of one event inside its stream.
    StreamSequence,
    1
);
sequence_identity!(
    /// Non-negative CAS revision for state mutation rows.
    Revision,
    0
);

impl StreamVersion {
    pub fn zero() -> Self {
        Self(0)
    }
}

/// The exact head version the batch expects for one stream it changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedStreamHead {
    pub stream_id: StreamId,
    pub expected: StreamVersion,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_id_namespaces() {
        let session = StreamId::agent_session("s-1").unwrap();
        assert_eq!(session.as_str(), "agent-session:s-1");
        assert_eq!(
            session.kind(),
            StreamKind::AgentSession {
                session_id: "s-1".to_string()
            }
        );
        let workflow = StreamId::workflow("exec.1").unwrap();
        assert_eq!(
            workflow.kind(),
            StreamKind::Workflow {
                execution_id: "exec.1".to_string()
            }
        );
        assert_eq!(StreamId::application().kind(), StreamKind::Application);
        assert!(StreamId::parse("agent-session:").is_err());
        assert!(StreamId::parse("other:x").is_err());
        assert!(StreamId::parse("agent-session:a b").is_err());
    }

    #[test]
    fn identity_bounds() {
        assert!(CommitIdentity::parse("").is_err());
        assert!(CommitIdentity::parse(&"a".repeat(128)).is_ok());
        assert!(CommitIdentity::parse(&"a".repeat(129)).is_err());
        assert!(CommitIdentity::parse("ok._:-09AZ").is_ok());
        assert!(CommitIdentity::parse("no/slash").is_err());
    }

    #[test]
    fn sequence_bounds() {
        assert!(GlobalSequence::new(0).is_err());
        assert!(GlobalSequence::new(1).is_ok());
        assert!(StreamVersion::new(-1).is_err());
        assert_eq!(StreamVersion::zero().value(), 0);
        let max = GlobalSequence::new(i64::MAX).unwrap();
        assert!(max.next().is_none());
        assert_eq!(GlobalSequence::new(1).unwrap().next().unwrap().value(), 2);
    }
}
