pub async fn observe<
    C: Send + Sync + 'static,
    T: Clone + PartialEq + Send + Sync + 'static,
    F: std::future::Future<Output = ()> + Send,
>(
    mut sources: tokio::sync::watch::Receiver<Option<std::sync::Arc<C>>>,
    settings: impl Fn(&C) -> (tokio::sync::watch::Receiver<Option<T>>, Option<T>)
        + Send
        + Sync
        + 'static,
    changed: impl Fn(&std::sync::Arc<C>, T) -> F + Send + Sync + 'static,
) {
    loop {
        let source = sources.borrow_and_update().clone();
        let Some(client) = source else {
            if sources.changed().await.is_err() {
                return;
            }
            continue;
        };
        let (mut source, initial) = settings(&client);
        let current = source.borrow_and_update().clone();
        if current != initial {
            if let Some(value) = current {
                changed(&client, value).await;
            }
        }
        loop {
            tokio::select! {
                update = sources.changed() => {
                    if update.is_err() { return; }
                    break;
                }
                update = source.changed() => {
                    if update.is_err() {
                        if sources.changed().await.is_err() { return; }
                        break;
                    }
                    let value = source.borrow_and_update().clone();
                    if let Some(value) = value { changed(&client, value).await; }
                }
            }
        }
    }
}
