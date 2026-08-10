#[cfg(test)]
use std::path::Path;
use tauri::Manager;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;

#[cfg(test)]
pub(crate) fn append_required_events(
    data_dir: &Path,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    WorkflowEventLog::new(data_dir).append_batch(events)
}

pub(crate) fn append_required_events_for_app_as<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    operation_kind: crate::domain::local_event::CommitOperationKind,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    let Some(store) = app
        .try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
    else {
        #[cfg(test)]
        return append_required_events(
            &crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
                .map_err(|_| "failed to resolve app data dir".to_string())?,
            events,
        );
        #[cfg(not(test))]
        return Err("workflow SQLite event authority is not managed".to_string());
    };
    let store = store.inner().clone();
    let repository: std::sync::Arc<
        dyn crate::domain::local_event::LocalEventTransactionRepository,
    > = store.clone();
    WorkflowEventLog::with_authority(repository, store.installation_id().to_string())
        .append_batch_durable_with_mutations_blocking_as(operation_kind, events, Vec::new())
}

pub(crate) fn append_required_events_with_mutations_for_app_as<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    operation_kind: crate::domain::local_event::CommitOperationKind,
    events: &[WorkflowEvent],
    state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
) -> Result<(), String> {
    let Some(store) = app
        .try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
    else {
        #[cfg(test)]
        return append_required_events(
            &crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
                .map_err(|_| "failed to resolve app data dir".to_string())?,
            events,
        );
        #[cfg(not(test))]
        return Err("workflow SQLite event authority is not managed".to_string());
    };
    let store = store.inner().clone();
    let repository: std::sync::Arc<
        dyn crate::domain::local_event::LocalEventTransactionRepository,
    > = store.clone();
    WorkflowEventLog::with_authority(repository, store.installation_id().to_string())
        .append_batch_durable_with_mutations_blocking_as(operation_kind, events, state_mutations)
}

pub(crate) fn append_provider_stop_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    execution_id: &str,
    events: &[WorkflowEvent],
    provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
    state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
) -> Result<(), String> {
    let Some(store) = app
        .try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
    else {
        #[cfg(test)]
        return if provider_events.is_empty() {
            append_required_events(
                &crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
                    .map_err(|_| "failed to resolve app data dir".to_string())?,
                events,
            )
        } else {
            Err("provider/workflow atomic event authority is not managed".to_string())
        };
        #[cfg(not(test))]
        return Err("workflow SQLite event authority is not managed".to_string());
    };
    let store = store.inner().clone();
    let repository: std::sync::Arc<
        dyn crate::domain::local_event::LocalEventTransactionRepository,
    > = store.clone();
    WorkflowEventLog::with_authority(repository, store.installation_id().to_string())
        .append_provider_stop_batch_blocking(execution_id, events, provider_events, state_mutations)
}

pub(crate) fn commit_projection_with_mutations_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    execution_id: &str,
    state_mutations: Vec<crate::domain::local_event::LocalStateMutation>,
) -> Result<(), String> {
    let Some(store) = app
        .try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
    else {
        #[cfg(test)]
        return Ok(());
        #[cfg(not(test))]
        return Err("workflow SQLite event authority is not managed".to_string());
    };
    let store = store.inner().clone();
    let repository: std::sync::Arc<
        dyn crate::domain::local_event::LocalEventTransactionRepository,
    > = store.clone();
    WorkflowEventLog::with_authority(repository, store.installation_id().to_string())
        .commit_projection_durable_blocking(execution_id, state_mutations)
}

pub(crate) fn read_events_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    execution_id: &str,
) -> Result<Vec<WorkflowEvent>, String> {
    if let Some(store) = app
        .try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
    {
        let store = store.inner().clone();
        let repository: std::sync::Arc<
            dyn crate::domain::local_event::LocalEventTransactionRepository,
        > = store.clone();
        return WorkflowEventLog::with_authority(repository, store.installation_id().to_string())
            .read_log_durable_blocking(execution_id);
    }
    #[cfg(test)]
    return WorkflowEventLog::new(
        &crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(|_| "failed to resolve app data dir".to_string())?,
    )
    .read_log(execution_id);
    #[cfg(not(test))]
    Err("workflow SQLite event authority is not managed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_required_events_writes_batch_to_execution_log() {
        let tmp = tempfile::tempdir().unwrap();
        let execution_id = "00000000-0000-0000-0000-000000000001";
        let event = WorkflowEvent::ExecutionAborted {
            execution_id: execution_id.to_string(),
            aborted_node: None,
            timestamp: 42.0,
        };

        append_required_events(tmp.path(), std::slice::from_ref(&event)).unwrap();

        let events = WorkflowEventLog::new(tmp.path())
            .read_log(execution_id)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            WorkflowEvent::ExecutionAborted {
                execution_id: restored_execution_id,
                timestamp,
                ..
            } if restored_execution_id == execution_id
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn append_required_events_delegates_empty_batch_as_noop() {
        let tmp = tempfile::tempdir().unwrap();

        append_required_events(tmp.path(), &[]).unwrap();

        assert!(!tmp.path().join("workflow_execution_logs").exists());
    }
}
