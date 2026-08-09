use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;

use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderKind, ProviderLifecycleBinding,
    ProviderLifecycleCredentialGateway, ProviderLifecycleEvent, ProviderLifecycleEventRepository,
    ProviderLifecycleIngressResult, ProviderLifecycleOutcome, ProviderLifecycleRejection,
    ProviderLifecycleRepositoryError, ProviderLifecycleScope, ProviderLifecycleSignal,
    ProviderLifecycleSlot, ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation,
    ScopedProviderLifecycleEvent,
};

mod hook_health;
mod ingress;
pub(crate) use hook_health::{
    ProviderHookHealthFailureObservation, ProviderHookHealthFailureQuery,
    ProviderHookHealthFailureQueryError, ProviderHookHealthReadUsecase, ProviderHookHealthUsecase,
    ProviderHookHealthUsecaseError, ProviderHookHealthWarning,
};
pub(crate) use ingress::{
    ProviderLifecycleIngressPort, ProviderLifecycleIngressUsecase,
    ProviderLifecycleIngressUsecaseError, ProviderSessionStartTransaction,
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleUsecaseError {
    #[error("Provider lifecycle input is invalid")]
    InvalidInput,
    #[error("Provider lifecycle persistence is unavailable")]
    StorageUnavailable,
    #[error("Provider lifecycle state is corrupt")]
    Corrupt,
}

impl From<ProviderLifecycleRepositoryError> for ProviderLifecycleUsecaseError {
    fn from(error: ProviderLifecycleRepositoryError) -> Self {
        match error {
            ProviderLifecycleRepositoryError::InvalidInput => Self::InvalidInput,
            ProviderLifecycleRepositoryError::StorageUnavailable => Self::StorageUnavailable,
            ProviderLifecycleRepositoryError::Corrupt => Self::Corrupt,
        }
    }
}

type LiveSlot = Arc<AsyncMutex<ProviderLifecycleSlot>>;

pub(crate) struct ProviderLifecycleUsecase {
    credentials: Arc<dyn ProviderLifecycleCredentialGateway>,
    events: Arc<dyn ProviderLifecycleEventRepository>,
    slots: Mutex<HashMap<ProviderLifecycleSlotId, LiveSlot>>,
}

impl ProviderLifecycleUsecase {
    pub(crate) fn new(
        credentials: Arc<dyn ProviderLifecycleCredentialGateway>,
        events: Arc<dyn ProviderLifecycleEventRepository>,
    ) -> Self {
        Self {
            credentials,
            events,
            slots: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn arm(
        &self,
        slot_id: ProviderLifecycleSlotId,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
    ) -> Result<ArmedProviderLifecycle, ProviderLifecycleUsecaseError> {
        let issued = self.credentials.issue();
        let (binding_id, capability, capability_hash) = issued.into_parts();
        let binding = ProviderLifecycleBinding::arm(&binding_id, provider, scope.clone())
            .map_err(|_| ProviderLifecycleUsecaseError::InvalidInput)?;
        let live = self.slot_for_arm(&slot_id)?;
        let mut current = live.lock().await;
        let mut candidate = current.clone();
        let facts = candidate.arm(binding, capability_hash);
        if let Err(error) = self.events.append(facts).await {
            drop(current);
            self.cleanup_empty_slot(&slot_id, &live)?;
            return Err(error.into());
        }
        *current = candidate;
        drop(current);
        Ok(ArmedProviderLifecycle::new(
            slot_id, binding_id, capability, provider, scope,
        ))
    }

    pub(crate) async fn arm_with_commit<T, E, F, Fut>(
        &self,
        slot_id: ProviderLifecycleSlotId,
        provider: ProviderKind,
        scope: ProviderLifecycleScope,
        commit: F,
    ) -> Result<(ArmedProviderLifecycle, T), E>
    where
        E: From<ProviderLifecycleUsecaseError>,
        F: FnOnce(Vec<ScopedProviderLifecycleEvent>) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let issued = self.credentials.issue();
        let (binding_id, capability, capability_hash) = issued.into_parts();
        let binding = ProviderLifecycleBinding::arm(&binding_id, provider, scope.clone())
            .map_err(|_| E::from(ProviderLifecycleUsecaseError::InvalidInput))?;
        let live = self.slot_for_arm(&slot_id).map_err(E::from)?;
        let mut current = live.lock().await;
        let mut candidate = current.clone();
        let facts = candidate.arm(binding, capability_hash);
        let committed = match commit(facts).await {
            Ok(committed) => committed,
            Err(error) => {
                drop(current);
                self.cleanup_empty_slot(&slot_id, &live).map_err(E::from)?;
                return Err(error);
            }
        };
        *current = candidate;
        drop(current);
        Ok((
            ArmedProviderLifecycle::new(slot_id, binding_id, capability, provider, scope),
            committed,
        ))
    }

    pub(crate) async fn receive(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        signal: ProviderLifecycleSignal,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleUsecaseError> {
        let Some(live) = self.find_slot(slot_id)? else {
            return Ok(ProviderLifecycleIngressResult::Rejected(
                ProviderLifecycleRejection::BindingNotActive,
            ));
        };
        let scope = signal.scope().clone();
        let capability_hash = self.credentials.hash(capability);
        let mut current = live.lock().await;
        let mut candidate = current.clone();
        let outcome = candidate.receive(&capability_hash, signal);
        let result = self
            .persist_outcome(&mut current, candidate, scope, outcome)
            .await;
        drop(current);
        self.cleanup_empty_slot(slot_id, &live)?;
        result
    }

    pub(crate) async fn receive_session_started_with_commit<E, F, Fut>(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        signal: ProviderLifecycleSignal,
        commit: F,
    ) -> Result<ProviderLifecycleIngressResult, E>
    where
        E: From<ProviderLifecycleUsecaseError>,
        F: FnOnce(Vec<ScopedProviderLifecycleEvent>) -> Fut,
        Fut: Future<Output = Result<(), E>>,
    {
        let Some(live) = self.find_slot(slot_id).map_err(E::from)? else {
            return Ok(ProviderLifecycleIngressResult::Rejected(
                ProviderLifecycleRejection::BindingNotActive,
            ));
        };
        let scope = signal.scope().clone();
        let capability_hash = self.credentials.hash(capability);
        let mut current = live.lock().await;
        let mut candidate = current.clone();
        let outcome = candidate.receive(&capability_hash, signal);
        let result = match outcome {
            ProviderLifecycleOutcome::Applied(events) => {
                commit(scoped(scope, events)).await?;
                *current = candidate;
                ProviderLifecycleIngressResult::Applied
            }
            ProviderLifecycleOutcome::Duplicate => {
                commit(Vec::new()).await?;
                ProviderLifecycleIngressResult::Duplicate
            }
            ProviderLifecycleOutcome::Rejected(reason) => {
                ProviderLifecycleIngressResult::Rejected(reason)
            }
        };
        drop(current);
        self.cleanup_empty_slot(slot_id, &live).map_err(E::from)?;
        Ok(result)
    }

    pub(crate) async fn report_unavailable(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        observation: ProviderLifecycleUnavailableObservation,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleUsecaseError> {
        let Some(live) = self.find_slot(slot_id)? else {
            return Ok(ProviderLifecycleIngressResult::Rejected(
                ProviderLifecycleRejection::BindingNotActive,
            ));
        };
        let scope = observation.scope().clone();
        let capability_hash = self.credentials.hash(capability);
        let mut current = live.lock().await;
        let mut candidate = current.clone();
        let outcome = candidate.report_unavailable(&capability_hash, observation);
        let result = self
            .persist_outcome(&mut current, candidate, scope, outcome)
            .await;
        drop(current);
        self.cleanup_empty_slot(slot_id, &live)?;
        result
    }

    pub(crate) async fn release(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        binding_id: &str,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleUsecaseError> {
        let Some(live) = self.find_slot(slot_id)? else {
            return Ok(ProviderLifecycleIngressResult::Duplicate);
        };
        let mut current = live.lock().await;
        let Some(scope) = current.current_scope().cloned() else {
            drop(current);
            self.cleanup_empty_slot(slot_id, &live)?;
            return Ok(ProviderLifecycleIngressResult::Duplicate);
        };
        let mut candidate = current.clone();
        let outcome = candidate.release(binding_id);
        let result = self
            .persist_outcome(&mut current, candidate, scope, outcome)
            .await;
        drop(current);
        self.cleanup_empty_slot(slot_id, &live)?;
        result
    }

    pub(crate) async fn release_scope(
        &self,
        scope: &ProviderLifecycleScope,
    ) -> Result<usize, ProviderLifecycleUsecaseError> {
        let slots = self
            .slots
            .lock()
            .map_err(|_| ProviderLifecycleUsecaseError::Corrupt)?
            .iter()
            .map(|(slot_id, live)| (slot_id.clone(), live.clone()))
            .collect::<Vec<_>>();
        let mut released = 0;
        for (slot_id, live) in slots {
            let mut current = live.lock().await;
            let mut candidate = current.clone();
            let Some(outcome) = candidate.release_scope(scope) else {
                continue;
            };
            let result = self
                .persist_outcome(&mut current, candidate, scope.clone(), outcome)
                .await?;
            drop(current);
            self.cleanup_empty_slot(&slot_id, &live)?;
            if result == ProviderLifecycleIngressResult::Applied {
                released += 1;
            }
        }
        Ok(released)
    }

    pub(crate) async fn active_launch_id(
        &self,
        provider: ProviderKind,
        scope: &ProviderLifecycleScope,
    ) -> Result<Option<ProviderLifecycleSlotId>, ProviderLifecycleUsecaseError> {
        let slots = self
            .slots
            .lock()
            .map_err(|_| ProviderLifecycleUsecaseError::Corrupt)?
            .iter()
            .map(|(slot_id, live)| (slot_id.clone(), live.clone()))
            .collect::<Vec<_>>();
        let mut matching = None;
        for (slot_id, live) in slots {
            let current = live.lock().await;
            if current.current_scope() != Some(scope)
                || current.current_provider() != Some(provider)
            {
                continue;
            }
            if matching.is_some() {
                return Err(ProviderLifecycleUsecaseError::Corrupt);
            }
            matching = Some(slot_id);
        }
        Ok(matching)
    }

    async fn persist_outcome(
        &self,
        current: &mut ProviderLifecycleSlot,
        candidate: ProviderLifecycleSlot,
        scope: ProviderLifecycleScope,
        outcome: ProviderLifecycleOutcome,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleUsecaseError> {
        match outcome {
            ProviderLifecycleOutcome::Applied(events) => {
                self.events.append(scoped(scope, events)).await?;
                *current = candidate;
                Ok(ProviderLifecycleIngressResult::Applied)
            }
            ProviderLifecycleOutcome::Duplicate => Ok(ProviderLifecycleIngressResult::Duplicate),
            ProviderLifecycleOutcome::Rejected(reason) => {
                Ok(ProviderLifecycleIngressResult::Rejected(reason))
            }
        }
    }

    fn slot_for_arm(
        &self,
        slot_id: &ProviderLifecycleSlotId,
    ) -> Result<LiveSlot, ProviderLifecycleUsecaseError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| ProviderLifecycleUsecaseError::Corrupt)?;
        Ok(slots
            .entry(slot_id.clone())
            .or_insert_with(|| {
                Arc::new(AsyncMutex::new(ProviderLifecycleSlot::new(slot_id.clone())))
            })
            .clone())
    }

    fn find_slot(
        &self,
        slot_id: &ProviderLifecycleSlotId,
    ) -> Result<Option<LiveSlot>, ProviderLifecycleUsecaseError> {
        self.slots
            .lock()
            .map(|slots| slots.get(slot_id).cloned())
            .map_err(|_| ProviderLifecycleUsecaseError::Corrupt)
    }

    fn cleanup_empty_slot(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        live: &LiveSlot,
    ) -> Result<(), ProviderLifecycleUsecaseError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| ProviderLifecycleUsecaseError::Corrupt)?;
        let Some(registered) = slots.get(slot_id) else {
            return Ok(());
        };
        if !Arc::ptr_eq(registered, live) || Arc::strong_count(live) != 2 {
            return Ok(());
        }
        let Ok(current) = live.try_lock() else {
            return Ok(());
        };
        if current.is_empty() {
            drop(current);
            slots.remove(slot_id);
        }
        Ok(())
    }

    #[cfg(test)]
    fn live_slot_count(&self) -> Result<usize, ProviderLifecycleUsecaseError> {
        self.slots
            .lock()
            .map(|slots| slots.len())
            .map_err(|_| ProviderLifecycleUsecaseError::Corrupt)
    }
}

fn scoped(
    scope: ProviderLifecycleScope,
    events: Vec<ProviderLifecycleEvent>,
) -> Vec<ScopedProviderLifecycleEvent> {
    events
        .into_iter()
        .map(|event| ScopedProviderLifecycleEvent::new(scope.clone(), event))
        .collect()
}

#[cfg(test)]
#[path = "provider_lifecycle_ingress_test.rs"]
mod provider_lifecycle_ingress_tests;
#[cfg(test)]
#[path = "provider_lifecycle_usecase_test.rs"]
mod provider_lifecycle_usecase_tests;
