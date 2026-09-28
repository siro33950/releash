#[path = "../../tests/state_subscription_reads/reads_test.rs"]
mod reads_tests;

pub(crate) use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
pub(crate) use crate::infrastructure::state_subscription::{Delivery, Event, Version};
pub(crate) use reads_tests::Fixture as StateReadsFixture;

pub(crate) fn start(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)?;
    usecase.start(client, &typed)?;
    usecase
        .test_presenter()
        .expect("test presenter")
        .start(client, target, cursor)
}

pub(crate) async fn start_read(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), crate::usecase::state_subscription::StateReadError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(crate::usecase::state_subscription::StateReadError::from_error)?;
    usecase
        .start_subscription(client, &typed, None, cursor)
        .await
}

pub(crate) fn stop(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)?;
    usecase.stop(client, &typed)?;
    usecase
        .publisher()
        .stop(client, &typed, &usecase.active_targets())
}

pub(crate) async fn stop_read(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)?;
    usecase.stop_subscription(client, &typed).await
}

pub(crate) async fn start_terminal(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    target: &str,
    cursor: Option<(&str, u64)>,
    input_id: &str,
) -> Result<(), crate::usecase::state_subscription::StateReadError> {
    use crate::usecase::state_subscription::StateReadError;
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(StateReadError::from_error)?;
    usecase
        .start_subscription(client, &typed, Some(input_id), cursor)
        .await
}

pub(crate) fn terminal_processed(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    target: &str,
    units: usize,
) -> Result<(), crate::usecase::state_subscription::StateReadError> {
    use crate::usecase::state_subscription::{StateReadError, StateReadFailure};
    if units
        != usecase
            .test_presenter()
            .expect("test presenter")
            .terminal_report_units()
    {
        return Err(StateReadError {
            source: StateReadFailure::InvalidTerminalInput,
            message: "Invalid terminal processed units".into(),
        });
    }
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(StateReadError::from_error)?;
    usecase.terminal_processed(client, &typed, units)
}

pub(crate) fn test_output() -> crate::usecase::state_subscription::StateSubscriptionOutputRef {
    crate::adaptor::presenter::state_subscription::test_output()
}

pub(crate) fn changes(
    output: &crate::usecase::state_subscription::StateSubscriptionOutputRef,
) -> tokio::sync::broadcast::Receiver<crate::usecase::state_subscription::StateChangeSource> {
    output
        .as_any()
        .downcast_ref::<StateSubscriptionPresenter>()
        .expect("test presenter")
        .change_sender()
        .subscribe()
}

pub(crate) fn same(
    value: &crate::adaptor::presenter::client::StatePayload,
    expected: impl std::borrow::Borrow<crate::usecase::state_subscription::StateValue>,
) -> bool {
    *value
        == crate::adaptor::presenter::state_subscription_wire::payload(expected.borrow()).unwrap()
}

pub(crate) fn payload(
    value: &crate::usecase::state_subscription::StateValue,
) -> Result<crate::adaptor::presenter::client::StatePayload, connectrpc::ConnectError> {
    crate::adaptor::presenter::state_subscription_wire::payload(value)
}

pub(crate) fn terminal_item(
    value: &crate::adaptor::presenter::client::StatePayload,
) -> &crate::adaptor::presenter::client::terminal_event::Item {
    use crate::adaptor::presenter::client::state_payload::Value;
    let Some(Value::Terminal(event)) = &value.value else {
        panic!("terminal payload");
    };
    event.item.as_ref().expect("terminal item")
}

use crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter;
use crate::usecase::state_subscription::{StateSubscriptionUsecase, SubscriptionError};
use futures_util::Stream;
use std::sync::Arc;

#[cfg(test)]
impl StateSubscriptionUsecase {
    pub(crate) fn new(
        paths: Vec<String>,
        timer: Arc<dyn crate::usecase::state_subscription::SubscriptionTimer>,
    ) -> Self {
        let presenter = Arc::new(StateSubscriptionPresenter::new(paths));
        Self::new_with_output(presenter.clone(), presenter.change_sender(), timer)
    }

    pub(crate) fn test_presenter(&self) -> Option<&StateSubscriptionPresenter> {
        self.output_ref().as_any().downcast_ref()
    }

    pub(crate) fn open(
        &self,
        id: String,
    ) -> Result<
        impl Stream<Item = crate::adaptor::presenter::state_subscription::StateSubscriptionEvent>
            + Send
            + use<>,
        SubscriptionError,
    > {
        self.test_presenter()
            .expect("test presenter")
            .stream(self.clone(), id)
    }
}
