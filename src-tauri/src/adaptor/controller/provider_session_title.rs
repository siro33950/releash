use crate::common::retry::{bounded, RetryBackoff};
use crate::usecase::agent_session::ProviderSessionTitleIngestionUsecase;
use crate::usecase::failure::{attempt_expired, FailureKey, ATTEMPT_LIMIT};
use crate::usecase::retry::Retrying;
use futures_util::{Stream, StreamExt};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub(crate) async fn run(
    retrying: Arc<Retrying>,
    usecase: Arc<ProviderSessionTitleIngestionUsecase>,
    mut ticks: impl Stream<Item = ()> + Unpin,
) {
    let claimed = Arc::new(Mutex::new(HashSet::new()));
    while ticks.next().await.is_some() {
        let due = retrying
            .restart(
                FailureKey::new("provider_session_title_list", "daemon"),
                RetryBackoff::ITEM,
                |_| bounded(ATTEMPT_LIMIT, attempt_expired, usecase.list_due()),
            )
            .await;
        let Ok(ids) = due else {
            return;
        };
        for id in ids {
            if !claimed.lock().expect("claimed sessions").insert(id.clone()) {
                continue;
            }
            let retrying = retrying.clone();
            let usecase = usecase.clone();
            let claimed = claimed.clone();
            tokio::spawn(async move {
                let result = retrying
                    .restart(
                        FailureKey::new("provider_session_title", &id),
                        RetryBackoff::ITEM,
                        |progress| {
                            bounded(
                                ATTEMPT_LIMIT,
                                attempt_expired,
                                usecase.ingest(&id, progress),
                            )
                        },
                    )
                    .await;
                if result.is_ok() {
                    claimed.lock().expect("claimed sessions").remove(&id);
                }
            });
        }
    }
}

#[cfg(test)]
#[path = "provider_session_title_test.rs"]
mod provider_session_title_tests;
