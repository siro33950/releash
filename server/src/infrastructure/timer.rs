use futures_util::Stream;
use std::time::Duration;

pub(crate) fn ticks(period: Duration) -> impl Stream<Item = ()> + Send {
    ticks_after(Duration::ZERO, period)
}

pub(crate) fn ticks_after(delay: Duration, period: Duration) -> impl Stream<Item = ()> + Send {
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + delay, period);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    futures_util::stream::unfold(timer, |mut timer| async move {
        timer.tick().await;
        Some(((), timer))
    })
}

pub(crate) type TimeStream = std::pin::Pin<Box<dyn Stream<Item = ()> + Send>>;
pub(crate) type Ticks = std::sync::Arc<dyn Fn() -> TimeStream + Send + Sync>;
pub(crate) type Delay = std::sync::Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync,
>;

pub fn delays(duration: Duration) -> Delay {
    std::sync::Arc::new(move || Box::pin(tokio::time::sleep(duration)))
}

#[cfg(test)]
#[path = "timer_test.rs"]
mod timer_tests;
