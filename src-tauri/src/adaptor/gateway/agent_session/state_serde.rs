use serde::{Deserialize, Serialize};

use crate::domain::agent_session::value_objects::{ContextCarryState, SessionState, TurnPhase};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SessionStateSerde {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

impl From<&SessionState> for SessionStateSerde {
    fn from(value: &SessionState) -> Self {
        match value {
            SessionState::Active => Self::Active,
            SessionState::Idle => Self::Idle,
            SessionState::Done => Self::Done,
            SessionState::Error => Self::Error,
            SessionState::Closed => Self::Closed,
            SessionState::Archived => Self::Archived,
        }
    }
}

impl From<SessionStateSerde> for SessionState {
    fn from(value: SessionStateSerde) -> Self {
        match value {
            SessionStateSerde::Active => Self::Active,
            SessionStateSerde::Idle => Self::Idle,
            SessionStateSerde::Done => Self::Done,
            SessionStateSerde::Error => Self::Error,
            SessionStateSerde::Closed => Self::Closed,
            SessionStateSerde::Archived => Self::Archived,
        }
    }
}

impl Serialize for SessionState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SessionStateSerde::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SessionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        SessionStateSerde::deserialize(deserializer).map(Self::from)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContextCarryStateSerde {
    Resumed,
    Reinjected,
    Failed,
}

impl From<&ContextCarryState> for ContextCarryStateSerde {
    fn from(value: &ContextCarryState) -> Self {
        match value {
            ContextCarryState::Resumed => Self::Resumed,
            ContextCarryState::Reinjected => Self::Reinjected,
            ContextCarryState::Failed => Self::Failed,
        }
    }
}

impl From<ContextCarryStateSerde> for ContextCarryState {
    fn from(value: ContextCarryStateSerde) -> Self {
        match value {
            ContextCarryStateSerde::Resumed => Self::Resumed,
            ContextCarryStateSerde::Reinjected => Self::Reinjected,
            ContextCarryStateSerde::Failed => Self::Failed,
        }
    }
}

impl Serialize for ContextCarryState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ContextCarryStateSerde::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ContextCarryState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        ContextCarryStateSerde::deserialize(deserializer).map(Self::from)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TurnPhaseSerde {
    Idle,
    Streaming,
    WaitingPermission,
}

impl From<TurnPhase> for TurnPhaseSerde {
    fn from(value: TurnPhase) -> Self {
        match value {
            TurnPhase::Idle => Self::Idle,
            TurnPhase::Streaming => Self::Streaming,
            TurnPhase::WaitingPermission => Self::WaitingPermission,
        }
    }
}

impl From<TurnPhaseSerde> for TurnPhase {
    fn from(value: TurnPhaseSerde) -> Self {
        match value {
            TurnPhaseSerde::Idle => Self::Idle,
            TurnPhaseSerde::Streaming => Self::Streaming,
            TurnPhaseSerde::WaitingPermission => Self::WaitingPermission,
        }
    }
}

impl Serialize for TurnPhase {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        TurnPhaseSerde::from(*self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TurnPhase {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        TurnPhaseSerde::deserialize(deserializer).map(Self::from)
    }
}
