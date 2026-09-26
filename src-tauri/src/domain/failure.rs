#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TechnicalFailureNature {
    Transient,
    TimedOut,
    Cancelled,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TechnicalFailure {
    pub nature: TechnicalFailureNature,
    pub message: String,
}
impl std::fmt::Display for TechnicalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusinessFailure {
    VersionConflict,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Business(BusinessFailure),
    Technical(TechnicalFailureNature),
}

#[cfg(test)]
#[path = "failure_test.rs"]
mod failure_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageFailure {
    pub nature: TechnicalFailureNature,
    pub source: StorageFailureSource,
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageFailureSource {
    Commit(crate::domain::local_event::CommitBatchError),
    Query(crate::domain::local_event::LocalEventQueryError),
    Technical(TechnicalFailure),
    Editor(crate::domain::external_editor::EditorError),
    Workflow(Box<crate::domain::workflow::WorkflowError>),
    Repository(crate::domain::repository::RepositoryError),
    AgentSession(Box<crate::domain::agent_session::repository::AgentSessionRepositoryError>),
}

impl StorageFailure {
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.context = Some(message.into());
        self
    }
}

impl std::fmt::Display for StorageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.context {
            Some(message) => f.write_str(message),
            None => match &self.source {
                StorageFailureSource::Commit(error) => error.fmt(f),
                StorageFailureSource::Query(error) => error.fmt(f),
                StorageFailureSource::Technical(error) => error.fmt(f),
                StorageFailureSource::Editor(error) => error.fmt(f),
                StorageFailureSource::Workflow(error) => error.fmt(f),
                StorageFailureSource::Repository(error) => error.fmt(f),
                StorageFailureSource::AgentSession(error) => write!(f, "{error:?}"),
            },
        }
    }
}
