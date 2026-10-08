use crate::usecase::terminal_surface::subscription::TerminalRefresh;

pub(crate) fn start() -> tokio::sync::mpsc::UnboundedSender<TerminalRefresh> {
    let (sender, mut requests) = tokio::sync::mpsc::unbounded_channel::<TerminalRefresh>();
    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            spawn_worker(request);
        }
    });
    sender
}

fn spawn_worker(request: TerminalRefresh) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let run =
            async move { while request.usecase.refresh_terminal_once(&request.target).await {} };
        tokio::select! { biased; _ = request.cancelled => {}, _ = run => {} }
    })
}

#[cfg(test)]
#[path = "terminal_subscription_test.rs"]
mod terminal_subscription_tests;
