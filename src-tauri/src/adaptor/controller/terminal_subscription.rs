use crate::usecase::terminal_surface::subscription::TerminalSubscriptionUsecase;

pub(crate) fn start(usecase: &TerminalSubscriptionUsecase) {
    let mut requests = usecase.take_refresh_events();
    let usecase = usecase.clone();
    tokio::spawn(async move {
        while let Some(request) = requests.recv().await {
            let usecase = usecase.clone();
            tokio::spawn(async move {
                let run =
                    async move { while usecase.refresh_terminal_once(&request.target).await {} };
                tokio::select! { biased; _ = request.cancelled => {}, _ = run => {} }
            });
        }
    });
}
