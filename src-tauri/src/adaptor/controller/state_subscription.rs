use crate::infrastructure::timer::Delay;
use crate::usecase::state_subscription::{ReadSignal, ReadWorker};
use futures_util::StreamExt;

pub(crate) fn drive(ticks: Delay) -> tokio::sync::mpsc::UnboundedSender<ReadWorker> {
    let (sender, mut requests) = tokio::sync::mpsc::unbounded_channel::<ReadWorker>();
    tokio::spawn(async move {
        while let Some(mut worker) = requests.recv().await {
            let mut interval = ticks();
            let mut interval_ended = false;
            tokio::spawn(async move {
                let cancelled = worker.cancelled;
                tokio::pin!(cancelled);
                let run = async move {
                    loop {
                        let signal = tokio::select! {
                            change = worker.changes.recv() => match change {
                                Ok(change) => ReadSignal::Change(change),
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => ReadSignal::Lagged,
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            },
                            waiting = worker.waiting_changes.recv() => match waiting {
                                Some(waiting) => ReadSignal::Waiting(waiting),
                                None => break,
                            },
                            tick = interval.next(), if !interval_ended => {
                                if tick.is_none() {
                                    interval_ended = true;
                                    continue;
                                }
                                ReadSignal::Periodic
                            }
                        };
                        worker
                            .usecase
                            .refresh_read(
                                &worker.target,
                                signal,
                                &mut worker.changes,
                                &mut worker.waiting_changes,
                            )
                            .await;
                    }
                };
                tokio::select! { biased; _ = cancelled => {}, _ = run => {} }
            });
        }
    });
    sender
}
