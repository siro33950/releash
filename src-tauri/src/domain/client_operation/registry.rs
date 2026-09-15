use std::collections::{HashMap, HashSet};

use super::policy::{self, OrderingScope, Recovery, MAX_UNACKNOWLEDGED_OPERATIONS};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationIdentity {
    pub command: String,
    pub fingerprint: [u8; 32],
    pub target: Option<(OrderingScope, [u8; 32])>,
}

#[derive(Clone, Debug)]
pub struct OperationReference {
    pub id: String,
    pub command: String,
    pub uncertain: bool,
    pub fingerprint: Option<[u8; 32]>,
    pub target: Option<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationDecision<R> {
    Pending,
    Completed(R),
    Unknown,
    Ready,
    Blocked,
    Bound(String),
    Conflict,
    Full,
    NotSent,
    Disconnected,
    WatchReleased,
}

enum RecordedState<R> {
    Pending,
    Completed {
        result: R,
        at_ms: u64,
        sequence: u64,
    },
}

impl<R: Clone> RecordedState<R> {
    fn decision(&self) -> OperationDecision<R> {
        match self {
            Self::Pending => OperationDecision::Pending,
            Self::Completed { result, .. } => OperationDecision::Completed(result.clone()),
        }
    }
}

enum WatchState {
    Starting(Option<String>),
    Unreceived { connection: String, id: u64 },
}

#[derive(Default)]
pub struct RecoveryAttempt<'a> {
    pub connection: &'a str,
    pub sent: bool,
    pub expired: bool,
    pub user_retry: bool,
    pub successors: &'a [OperationReference],
}

struct Operation<R> {
    generation: String,
    identity: OperationIdentity,
    state: RecordedState<R>,
    watch: Option<WatchState>,
}

pub struct OperationRegistry<R> {
    generation: String,
    operations: HashMap<String, Operation<R>>,
    now_ms: u64,
    // ponytail: 破棄したIDだけ世代内で保持する。件数が問題になれば永続admission索引へ移す。
    discarded: HashSet<String>,
    completed_sequence: u64,
    watches: HashMap<String, (u64, OperationIdentity, R)>,
    released_watches: Vec<u64>,
}

impl<R: Clone> OperationRegistry<R> {
    pub fn new(generation: String) -> Self {
        Self {
            generation,
            operations: HashMap::new(),
            now_ms: 0,
            discarded: HashSet::new(),
            completed_sequence: 0,
            watches: HashMap::new(),
            released_watches: Vec::new(),
        }
    }

    pub fn query(&self, id: &str, generation: &str) -> OperationDecision<R> {
        if !generation.is_empty()
            && generation != self.generation
            && self
                .operations
                .get(id)
                .is_none_or(|operation| operation.generation != generation)
        {
            return OperationDecision::Unknown;
        }
        if let Some((_, _, result)) = self.watches.get(id) {
            return OperationDecision::Completed(result.clone());
        }
        self.operations
            .get(id)
            .map_or(OperationDecision::Unknown, |operation| {
                operation.state.decision()
            })
    }

    pub fn prepare(
        &self,
        id: &str,
        generation: &str,
        identity: &OperationIdentity,
        predecessors: &[OperationReference],
        attempt: &RecoveryAttempt<'_>,
    ) -> OperationDecision<R> {
        use OperationDecision::*;
        let same_generation = generation.is_empty() || generation == self.generation;
        if let Some(known) = self.operations.get(id) {
            return if known.identity == *identity {
                known.state.decision()
            } else {
                Conflict
            };
        }
        if let Some((_, known_identity, result)) = self.watches.get(id) {
            return if known_identity == identity {
                Completed(result.clone())
            } else {
                Conflict
            };
        }
        if !matches!(
            policy::recovery(&identity.command),
            Recovery::Read | Recovery::Connection
        ) {
            for previous in predecessors {
                if previous.id == id {
                    continue;
                }
                let known = self.operations.get(&previous.id);
                if previous.uncertain
                    && previous.command == identity.command
                    && known.map_or(
                        previous.fingerprint == Some(identity.fingerprint),
                        |operation| operation.identity.fingerprint == identity.fingerprint,
                    )
                {
                    return Bound(previous.id.clone());
                }
                if let Some((scope, _)) = identity.target {
                    let relevant = known.map_or(
                        policy::ordering_scope(&previous.command) == Some(scope)
                            && previous
                                .target
                                .is_none_or(|target| Some((scope, target)) == identity.target),
                        |operation| operation.identity.target == identity.target,
                    );
                    if relevant
                        && !known.is_some_and(|operation| {
                            matches!(operation.state, RecordedState::Completed { .. })
                        })
                    {
                        return Blocked;
                    }
                }
            }
        }
        if attempt.sent
            && attempt.successors.iter().any(|later| {
                identity.target.is_some_and(|(scope, target)| {
                    policy::ordering_scope(&later.command) == Some(scope)
                        && later.target == Some(target)
                })
            })
        {
            return Unknown;
        }
        if attempt.sent
            && policy::is_watch(&identity.command)
            && (attempt.expired || self.discarded.contains(id))
        {
            return WatchReleased;
        }
        if attempt.expired {
            return if !attempt.sent {
                NotSent
            } else if policy::recovery(&identity.command) == Recovery::Read {
                Disconnected
            } else {
                Unknown
            };
        }
        if same_generation
            && self.discarded.contains(id)
            && policy::recovery(&identity.command) != Recovery::Read
            && !(attempt.user_retry
                && policy::recovery(&identity.command) == Recovery::CallerAttempt)
        {
            return Unknown;
        }
        if attempt.user_retry && same_generation && !self.discarded.contains(id) {
            return Ready;
        }
        if (!same_generation || attempt.sent)
            && !policy::may_replay(&identity.command, same_generation, attempt.expired)
        {
            return if policy::recovery(&identity.command) == Recovery::Connection {
                Disconnected
            } else {
                Unknown
            };
        }
        Ready
    }

    pub fn admit(
        &mut self,
        id: &str,
        generation: &str,
        identity: OperationIdentity,
        predecessors: &[OperationReference],
        attempt: &RecoveryAttempt<'_>,
    ) -> OperationDecision<R> {
        if !generation.is_empty() && generation != self.generation && !attempt.sent {
            return OperationDecision::Unknown;
        }
        let decision = self.prepare(id, generation, &identity, predecessors, attempt);
        if !matches!(decision, OperationDecision::Ready) {
            return if matches!(decision, OperationDecision::NotSent) {
                OperationDecision::Unknown
            } else {
                decision
            };
        }
        if matches!(
            policy::recovery(&identity.command),
            Recovery::Read | Recovery::Connection
        ) && !policy::is_watch(&identity.command)
        {
            return OperationDecision::Ready;
        }
        if self.operations.len() >= MAX_UNACKNOWLEDGED_OPERATIONS {
            let oldest = self
                .operations
                .iter()
                .filter_map(|(id, operation)| match &operation.state {
                    RecordedState::Completed { sequence, .. } => Some((id.clone(), *sequence)),
                    _ => None,
                })
                .min_by_key(|(_, at_ms)| *at_ms);
            if let Some((id, _)) = oldest {
                self.discard(&id);
            } else {
                return OperationDecision::Full;
            }
        }
        let watch = policy::is_watch(&identity.command)
            .then(|| WatchState::Starting(Some(attempt.connection.into())));
        self.operations.insert(
            id.into(),
            Operation {
                generation: generation.into(),
                identity,
                state: RecordedState::Pending,
                watch,
            },
        );
        OperationDecision::Ready
    }

    fn discard(&mut self, id: &str) {
        self.discarded.insert(id.into());
        if let Some(Operation {
            watch: Some(WatchState::Unreceived { id, .. }),
            ..
        }) = self.operations.remove(id)
        {
            self.released_watches.push(id);
        }
    }

    pub fn advance(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
        let expired: Vec<_> = self
            .operations
            .iter()
            .filter(|(_, operation)| {
                matches!(operation.state, RecordedState::Completed { at_ms, .. }
                if now_ms.saturating_sub(at_ms) >= policy::OPERATION_RETENTION_MS)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.discard(&id);
        }
    }

    pub fn take_released_watches(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.released_watches)
    }

    pub fn complete(&mut self, id: &str, result: &R, watcher_id: Option<u64>) -> Option<u64> {
        let operation = self.operations.get_mut(id)?;
        if !matches!(operation.state, RecordedState::Pending) {
            return None;
        }
        if let Some(WatchState::Starting(connection)) = &operation.watch {
            if let Some(watcher_id) = watcher_id {
                let Some(connection) = connection.clone() else {
                    self.operations.remove(id);
                    return Some(watcher_id);
                };
                operation.watch = Some(WatchState::Unreceived {
                    connection,
                    id: watcher_id,
                });
            } else {
                operation.watch = None;
            }
        }
        self.completed_sequence += 1;
        operation.state = RecordedState::Completed {
            result: result.clone(),
            at_ms: self.now_ms,
            sequence: self.completed_sequence,
        };
        None
    }

    pub fn forget_watch(&mut self, watcher_id: u64) {
        self.operations.retain(|_, operation| {
            !matches!(operation.watch,
            Some(WatchState::Unreceived { id, .. }) if id == watcher_id)
        });
        self.watches.retain(|_, (id, _, _)| *id != watcher_id);
    }

    pub fn watch_active(&self, id: &str) -> bool {
        self.watches.contains_key(id)
    }

    pub fn acknowledge(&mut self, id: &str, release_watch: bool) -> Option<u64> {
        if release_watch {
            if let Some((watcher_id, _, _)) = self.watches.remove(id) {
                return Some(watcher_id);
            }
        }
        let operation = self.operations.get_mut(id)?;
        if !matches!(operation.state, RecordedState::Completed { .. }) {
            return None;
        }
        if let Some(WatchState::Unreceived { id: watcher_id, .. }) = operation.watch {
            if !release_watch {
                let RecordedState::Completed { result, .. } = &operation.state else {
                    unreachable!()
                };
                self.watches.insert(
                    id.into(),
                    (watcher_id, operation.identity.clone(), result.clone()),
                );
                self.operations.remove(id);
                return None;
            }
            self.operations.remove(id);
            return Some(watcher_id);
        }
        if operation.watch.is_none() {
            self.operations.remove(id);
        }
        None
    }

    pub fn disconnect(&mut self, connection: &str) -> Vec<u64> {
        let mut release = Vec::new();
        self.operations
            .retain(|_, operation| match &mut operation.watch {
                Some(WatchState::Starting(owner)) if owner.as_deref() == Some(connection) => {
                    *owner = None;
                    true
                }
                Some(WatchState::Unreceived {
                    connection: owner,
                    id,
                }) if owner == connection => {
                    release.push(*id);
                    false
                }
                _ => true,
            });
        release
    }
}

#[cfg(test)]
#[path = "registry_test.rs"]
mod registry_tests;
