use std::sync::Arc;

use crate::adaptor::protocol::agent_session_v1::{
    decode_nonnegative_i64_decimal, decode_nonnegative_u64_decimal, ApplicationQuitErrorDtoV1,
    ApplicationQuitLookupErrorDtoV1, CurrentShutdownErrorDtoV1, MigrationQueryErrorDtoV1,
    OperationApplicationErrorDtoV1, RecoveryActionCommandErrorDtoV1, RecoveryActionOutcomeDtoV1,
    ShutdownDetailsMutationErrorDtoV1, ShutdownPlanQueryErrorDtoV1,
};
use crate::adaptor::protocol::application_lifecycle_v1::{
    ApplicationQuitIntentDtoV1, ApplicationQuitLookupDtoV1, ApplicationQuitOutcomeDtoV1,
    ApplicationQuitRequestDtoV1, CurrentShutdownResultDtoV1, LocalStoreMigrationResultDtoV1,
    ShutdownPlanDtoV1, ShutdownPlanPageDtoV1, ShutdownTargetActionRequestDtoV1,
};

use crate::domain::local_event::ShutdownPlanKey;
use crate::usecase::shutdown_coordinator::{
    ApplicationQuitIntent, ApplicationQuitRequest, ShutdownCoordinator, ShutdownTargetActionRequest,
};

pub(crate) fn query_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> OperationApplicationErrorDtoV1 {
    crate::adaptor::presenter::application_lifecycle::query_error(error)
}

fn shutdown_recovery_action_error(
    error: crate::usecase::agent_session::operation::RecoveryActionError,
) -> RecoveryActionCommandErrorDtoV1 {
    use crate::usecase::agent_session::operation::RecoveryActionError as E;
    match error {
        E::InvalidRequest => RecoveryActionCommandErrorDtoV1::InvalidRequest,
        E::ShutdownInProgress => RecoveryActionCommandErrorDtoV1::ShutdownInProgress,
        E::NotFound => RecoveryActionCommandErrorDtoV1::NotFound,
        E::StorageUnavailable { failure } => RecoveryActionCommandErrorDtoV1::StorageUnavailable {
            failure: failure.into(),
        },
        E::Internal { correlation_id } => {
            RecoveryActionCommandErrorDtoV1::Internal { correlation_id }
        }
        other => RecoveryActionCommandErrorDtoV1::Internal {
            correlation_id: presentation_correlation(
                "shutdown_recovery_action",
                &format!("{other:?}"),
            ),
        },
    }
}

fn presentation_correlation(context: &str, detail: &str) -> String {
    match crate::adaptor::presenter::application_lifecycle::presentation_error(context, detail) {
        OperationApplicationErrorDtoV1::Internal { correlation_id } => correlation_id,
        _ => unreachable!("presentation errors are always Internal"),
    }
}

#[tauri::command]
pub(crate) async fn request_application_quit(
    app: tauri::AppHandle,
    coordinator: tauri::State<'_, Arc<ShutdownCoordinator>>,
    process_actions: tauri::State<
        '_,
        Arc<crate::adaptor::controller::application_lifecycle::ApplicationProcessActionDispatcher>,
    >,
    request: ApplicationQuitRequestDtoV1,
) -> Result<ApplicationQuitOutcomeDtoV1, ApplicationQuitErrorDtoV1> {
    let (dto, process_action) =
        request_application_quit_result(coordinator.inner().as_ref(), request).await?;
    if let Some(process_action) = process_action {
        process_actions.dispatch_tauri(app, process_action);
    }
    Ok(dto)
}

pub(crate) async fn request_application_quit_result(
    coordinator: &ShutdownCoordinator,
    request: ApplicationQuitRequestDtoV1,
) -> Result<
    (
        ApplicationQuitOutcomeDtoV1,
        Option<crate::usecase::shutdown_coordinator::ApplicationProcessAction>,
    ),
    ApplicationQuitErrorDtoV1,
> {
    let intent = match request.intent {
        ApplicationQuitIntentDtoV1::Exit { code } => ApplicationQuitIntent::Exit { code },
        ApplicationQuitIntentDtoV1::Restart { code } => ApplicationQuitIntent::Restart { code },
    };
    let outcome = match coordinator
        .request(ApplicationQuitRequest {
            principal: crate::adaptor::controller::agent_session_operation_wiring::LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
            request_id: request.request_id,
            intent,
        })
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            return crate::adaptor::presenter::application_lifecycle::application_quit_failure(
                error,
            )
            .map(|outcome| (outcome, None));
        }
    };
    Ok(crate::adaptor::presenter::application_lifecycle::application_quit_outcome(outcome))
}

#[tauri::command]
pub(crate) async fn get_application_quit_operation(
    coordinator: tauri::State<'_, Arc<ShutdownCoordinator>>,
    operation_id: String,
) -> Result<ApplicationQuitLookupDtoV1, ApplicationQuitLookupErrorDtoV1> {
    get_application_quit_operation_result(coordinator.inner().as_ref(), operation_id).await
}

pub(crate) async fn get_application_quit_operation_result(
    coordinator: &ShutdownCoordinator,
    operation_id: String,
) -> Result<ApplicationQuitLookupDtoV1, ApplicationQuitLookupErrorDtoV1> {
    let value = coordinator
        .get_application_quit_projection(&operation_id)
        .await
        .map_err(crate::adaptor::presenter::application_lifecycle::application_quit_lookup_error)?;
    value
        .map(crate::adaptor::presenter::application_lifecycle::application_quit_lookup)
        .ok_or(ApplicationQuitLookupErrorDtoV1::NotFound)
}

#[tauri::command]
pub(crate) async fn resolve_shutdown_target_action(
    app: tauri::AppHandle,
    coordinator: tauri::State<'_, Arc<ShutdownCoordinator>>,
    process_actions: tauri::State<
        '_,
        Arc<crate::adaptor::controller::application_lifecycle::ApplicationProcessActionDispatcher>,
    >,
    request: ShutdownTargetActionRequestDtoV1,
) -> Result<RecoveryActionOutcomeDtoV1, RecoveryActionCommandErrorDtoV1> {
    let epoch = decode_nonnegative_i64_decimal(&request.epoch)
        .ok_or(RecoveryActionCommandErrorDtoV1::InvalidRequest)?;
    let ordinal = decode_nonnegative_i64_decimal(&request.ordinal)
        .ok_or(RecoveryActionCommandErrorDtoV1::InvalidRequest)?;
    let origin_revision = decode_nonnegative_u64_decimal(&request.origin_revision)
        .ok_or(RecoveryActionCommandErrorDtoV1::InvalidRequest)?;
    let execution = coordinator
        .resolve_shutdown_target_action(ShutdownTargetActionRequest {
            action_id: request.action_id,
            plan: ShutdownPlanKey {
                plan_id: request.plan_id,
                epoch,
            },
            ordinal,
            target_key: request.target_key,
            origin_revision,
            action: request.action.into(),
        })
        .await
        .map_err(shutdown_recovery_action_error)?;
    let outcome =
        if let crate::usecase::agent_session::operation::RecoveryActionOutcome::Completed {
            ref action_id,
            ..
        } = execution.outcome
        {
            let status = coordinator
                .get_shutdown_target_action_status(action_id)
                .await
                .map_err(shutdown_recovery_action_error)?;
            RecoveryActionOutcomeDtoV1::from_durable_status(status)
        } else {
            execution.outcome.into()
        };
    if let Some(process_action) = execution.process_action {
        process_actions.dispatch_tauri(app, process_action);
    }
    Ok(outcome)
}

#[tauri::command]
pub(crate) async fn get_application_shutdown(
    coordinator: tauri::State<'_, Arc<ShutdownCoordinator>>,
) -> Result<CurrentShutdownResultDtoV1, CurrentShutdownErrorDtoV1> {
    get_application_shutdown_result(coordinator.inner().as_ref()).await
}

pub(crate) async fn get_application_shutdown_result(
    coordinator: &ShutdownCoordinator,
) -> Result<CurrentShutdownResultDtoV1, CurrentShutdownErrorDtoV1> {
    let value = coordinator
        .current_application_shutdown_projection()
        .await
        .map_err(crate::adaptor::presenter::application_lifecycle::current_shutdown_error)?;
    Ok(crate::adaptor::presenter::application_lifecycle::current_shutdown(value))
}

#[tauri::command]
pub(crate) async fn get_shutdown_plan(
    coordinator: tauri::State<'_, Arc<ShutdownCoordinator>>,
    plan_id: String,
    epoch: String,
    limit: Option<usize>,
    cursor: Option<String>,
) -> Result<ShutdownPlanPageDtoV1, ShutdownPlanQueryErrorDtoV1> {
    get_shutdown_plan_result(coordinator.inner().as_ref(), plan_id, epoch, limit, cursor).await
}

pub(crate) async fn get_shutdown_plan_result(
    coordinator: &ShutdownCoordinator,
    plan_id: String,
    epoch: String,
    limit: Option<usize>,
    cursor: Option<String>,
) -> Result<ShutdownPlanPageDtoV1, ShutdownPlanQueryErrorDtoV1> {
    let epoch = decode_nonnegative_i64_decimal(&epoch)
        .ok_or(ShutdownPlanQueryErrorDtoV1::InvalidRequest)?;
    let page = coordinator
        .shutdown_plan_page_read_model(
            ShutdownPlanKey { plan_id, epoch },
            limit.unwrap_or(128),
            cursor,
        )
        .await
        .map_err(crate::adaptor::presenter::application_lifecycle::shutdown_plan_query_error)?;
    crate::adaptor::presenter::application_lifecycle::checked_plan_page(page)
        .map_err(crate::adaptor::presenter::application_lifecycle::shutdown_plan_query_error)
}

#[tauri::command]
pub(crate) async fn get_local_store_migration(
    coordinator: tauri::State<'_, Arc<ShutdownCoordinator>>,
) -> Result<LocalStoreMigrationResultDtoV1, MigrationQueryErrorDtoV1> {
    let migration = coordinator
        .local_store_migration_read_model()
        .await
        .map_err(crate::adaptor::presenter::application_lifecycle::migration_query_error)?
        .map(crate::adaptor::presenter::application_lifecycle::migration);
    Ok(LocalStoreMigrationResultDtoV1::Current { migration })
}

#[tauri::command]
pub(crate) async fn compact_application_shutdown_details(
    coordinator: tauri::State<'_, Arc<ShutdownCoordinator>>,
    plan_id: String,
    epoch: String,
) -> Result<ShutdownPlanDtoV1, ShutdownDetailsMutationErrorDtoV1> {
    let epoch = decode_nonnegative_i64_decimal(&epoch)
        .ok_or(ShutdownDetailsMutationErrorDtoV1::InvalidRequest)?;
    let compacted = coordinator
        .compact_shutdown_details_read_model(ShutdownPlanKey { plan_id, epoch })
        .await
        .map_err(
            crate::adaptor::presenter::application_lifecycle::shutdown_details_mutation_error,
        )?;
    Ok(crate::adaptor::presenter::application_lifecycle::plan(
        compacted,
    ))
}

pub(super) const COMMAND_NAMES: &[&str] = &[
    "request_application_quit",
    "get_application_quit_operation",
    "get_application_shutdown",
    "get_shutdown_plan",
    "resolve_shutdown_target_action",
    "get_local_store_migration",
    "compact_application_shutdown_details",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

fn invoke_handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        request_application_quit,
        get_application_quit_operation,
        get_application_shutdown,
        get_shutdown_plan,
        resolve_shutdown_target_action,
        get_local_store_migration,
        compact_application_shutdown_details,
    ]
}
