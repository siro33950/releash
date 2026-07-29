fn publish_session_notice(
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    notice: SessionNotice,
) {
    status_notifier.status_changed(status_center.record_session_notice(notice.clone()));
    notifier.persist_notice(notice);
}

fn report_event_log_recovered(
    status_center: &Arc<AgentStatusCenter>,
    status_notifier: &Arc<dyn AgentStatusNotifier>,
    notifier: &Arc<dyn AgentSessionEventNotifier>,
    session_id: &str,
) {
    emit_persistence_log_record(PersistenceLogRecord::EventLogRecovered {
        session_id: session_id.to_string(),
        kind: "event_log_recovered",
    });
    publish_session_notice(
        status_center,
        status_notifier,
        notifier,
        SessionNotice {
            session_id: session_id.to_string(),
            kind: SessionNoticeKind::EventLogRecovered,
            message: "Recovered a damaged session event log. New messages can be saved again."
                .to_string(),
            created_at: crate::usecase::agent_session::session::now_timestamp(),
        },
    );
}

fn report_persist_failure(
    ctx: &RuntimeContext,
    session_id: &str,
    kind: PersistFailureKind,
    error: &str,
) {
    emit_persistence_log_record(PersistenceLogRecord::PersistFailure {
        session_id: session_id.to_string(),
        kind: kind.as_str(),
        attempts: PERSIST_MAX_ATTEMPTS,
        error: error.to_string(),
    });
    publish_session_notice(
        &ctx.status_center,
        &ctx.status_notifier,
        &ctx.notifier,
        SessionNotice {
            session_id: session_id.to_string(),
            kind: SessionNoticeKind::PersistFailure,
            message: kind.notice_message().to_string(),
            created_at: crate::usecase::agent_session::session::now_timestamp(),
        },
    );
}

fn clear_persist_failure(ctx: &RuntimeContext, session_id: &str) {
    let changes = ctx
        .status_center
        .clear_session_notice(session_id, SessionNoticeKind::PersistFailure);
    if !changes.is_empty() {
        ctx.status_notifier.status_changed(changes);
    }
}

async fn persist_with_retry<T>(
    ctx: &RuntimeContext,
    session_id: &str,
    kind: PersistFailureKind,
    mut operation: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let mut last_error = None;
    for attempt in 1..=PERSIST_MAX_ATTEMPTS {
        match operation() {
            Ok(value) => {
                clear_persist_failure(ctx, session_id);
                return Ok(value);
            }
            Err(error) => {
                log::warn!(
                    "agent_session_persist_retry session_id={} kind={} attempt={} max_attempts={} error={}",
                    session_id,
                    kind.as_str(),
                    attempt,
                    PERSIST_MAX_ATTEMPTS,
                    error
                );
                last_error = Some(error);
                if let Some(backoff) = PERSIST_RETRY_BACKOFFS.get(attempt - 1) {
                    tokio::time::sleep(*backoff).await;
                }
            }
        }
    }
    let error = last_error.expect("persist retry must execute at least once");
    report_persist_failure(ctx, session_id, kind, &error);
    Err(error)
}

#[cfg(test)]
async fn append_session_event_and_project_state_with_retry(
    ctx: &RuntimeContext,
    session_id: &str,
    kind: PersistFailureKind,
    event: AgentSessionEvent,
) -> Result<SessionState, String> {
    let projected_state = persist_with_retry(ctx, session_id, kind, || {
        ctx.session_store
            .append_session_event_and_project(&ctx.data_dir, session_id, event.clone())
    })
    .await?;
    persist_with_retry(ctx, session_id, kind, || {
        ctx.session_store
            .set_session_state(&ctx.data_dir, session_id, projected_state.clone())
    })
    .await?;
    Ok(projected_state)
}

#[cfg(not(test))]
const PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD: std::time::Duration =
    std::time::Duration::from_secs(60);
#[cfg(test)]
const PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD: std::time::Duration =
    std::time::Duration::from_millis(50);
#[cfg(not(test))]
const PERMISSION_REQUEST_OBSERVED_TTL: std::time::Duration = std::time::Duration::from_secs(20);
#[cfg(test)]
const PERMISSION_REQUEST_OBSERVED_TTL: std::time::Duration = std::time::Duration::from_millis(100);

fn maybe_mark_permission_wait_diagnostic(
    session_id: &str,
    state: &mut RuntimeSessionState,
    now: std::time::Instant,
) -> bool {
    let Some(diagnostic) = state.mark_permission_wait_diagnostic_if_due(
        now,
        PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD,
        PERMISSION_REQUEST_OBSERVED_TTL,
    ) else {
        return false;
    };
    log::warn!(
        "agent permission wait diagnostic: chat_session={} request_id={} elapsed_ms={} threshold_ms={} observed=false",
        session_id,
        diagnostic.request_id,
        diagnostic.elapsed.as_millis(),
        PERMISSION_WAIT_DIAGNOSTIC_THRESHOLD.as_millis()
    );
    true
}
