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
