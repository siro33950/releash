#[path = "../../tests/state_subscription_reads/reads_test.rs"]
mod reads_tests;

pub(crate) use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
pub(crate) use crate::infrastructure::state_subscription::{Delivery, Event, Version};
pub(crate) use reads_tests::Fixture as StateReadsFixture;

pub(crate) fn test_output() -> crate::usecase::state_subscription::StateSubscriptionOutputRef {
    crate::adaptor::presenter::state_subscription::test_output()
}
