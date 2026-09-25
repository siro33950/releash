use crate::adaptor::controller::terminal_surface_runtime::{
    TerminalSurfaceEventFaultController, TerminalSurfaceRuntime,
};
use crate::adaptor::protocol::terminal::{TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1};
use crate::domain::state_subscription::{Event, SubscriptionTarget};
use crate::usecase::state_subscription::{
    StateSubscriptionEvent, StateSubscriptionUsecase, StateValue,
};
use futures_util::{Stream, StreamExt};
use std::{path::PathBuf, pin::Pin, sync::Arc};

pub struct TerminalSubscriptionHarness {
    runtime: TerminalSurfaceRuntime,
    subscriptions: StateSubscriptionUsecase,
}

impl std::ops::Deref for TerminalSubscriptionHarness {
    type Target = TerminalSurfaceRuntime;
    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

impl TerminalSubscriptionHarness {
    pub fn new(
        queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        data_dir: PathBuf,
    ) -> Self {
        Self::compose(TerminalSurfaceRuntime::new(queue, data_dir))
    }

    pub fn new_with_data_dir_and_event_faults(
        queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        data_dir: PathBuf,
    ) -> (Self, TerminalSurfaceEventFaultController) {
        let (runtime, faults) =
            TerminalSurfaceRuntime::new_with_data_dir_and_event_faults(queue, data_dir);
        (Self::compose(runtime), faults)
    }

    fn compose(runtime: TerminalSurfaceRuntime) -> Self {
        let subscriptions = StateSubscriptionUsecase::new(
            vec![],
            Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
        )
        .with_terminal(runtime.application());
        Self {
            runtime,
            subscriptions,
        }
    }

    #[cfg(feature = "desktop")]
    pub(crate) fn subscriptions(&self) -> StateSubscriptionUsecase {
        self.subscriptions.clone()
    }

    pub async fn subscribe(
        &self,
        input_id: String,
        owner: TerminalSurfaceOwnerV1,
    ) -> Result<TerminalSubscription, String> {
        let target = SubscriptionTarget::Terminal(owner.try_into()?).to_string();
        let client = uuid::Uuid::new_v4().to_string();
        let stream = Box::pin(
            self.subscriptions
                .open(client.clone())
                .map_err(|e| e.to_string())?,
        );
        self.subscriptions
            .start_terminal(&client, &target, None, &input_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(TerminalSubscription {
            stream,
            subscriptions: self.subscriptions.clone(),
            client,
            target,
            processed: 0,
            report_units: 0,
        })
    }
}

pub struct TerminalSubscription {
    stream: Pin<Box<dyn Stream<Item = StateSubscriptionEvent> + Send>>,
    subscriptions: StateSubscriptionUsecase,
    client: String,
    target: String,
    processed: usize,
    report_units: usize,
}

impl TerminalSubscription {
    pub async fn next(&mut self) -> Option<TerminalSurfaceStreamItemV1> {
        while let Some(event) = self.stream.next().await {
            let StateSubscriptionEvent::Item(target, event) = event else {
                continue;
            };
            assert_eq!(target, self.target);
            let value = match event {
                Event::Snapshot(_, value) | Event::Change(_, _, value) => value,
                _ => continue,
            };
            let StateValue::Terminal(item) = value.as_ref() else {
                panic!("expected terminal state");
            };
            let item: TerminalSurfaceStreamItemV1 = item.clone().into();
            match &item {
                TerminalSurfaceStreamItemV1::Snapshot { .. } => {
                    let wire = crate::adaptor::protocol::client::TerminalEvent::from(item.clone());
                    let Some(crate::adaptor::protocol::client::terminal_event::Item::Snapshot(
                        snapshot,
                    )) = wire.item
                    else {
                        unreachable!()
                    };
                    self.report_units = snapshot.processed_report_units as usize;
                    self.processed = 0;
                }
                TerminalSurfaceStreamItemV1::Output { data, .. } => {
                    self.processed += data.encode_utf16().count();
                    assert!(self.report_units > 0);
                    while self.processed >= self.report_units {
                        self.processed -= self.report_units;
                        self.subscriptions
                            .terminal_processed(&self.client, &self.target, self.report_units)
                            .expect("report processed output");
                    }
                }
                _ => {}
            }
            return Some(item);
        }
        None
    }
}
