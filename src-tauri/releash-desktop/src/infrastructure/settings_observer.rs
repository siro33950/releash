pub async fn observe<T: Clone + Send + Sync + 'static>(
    mut sources: tokio::sync::watch::Receiver<Option<tokio::sync::watch::Receiver<Option<T>>>>,
    changed: impl Fn(T) + Send + Sync + 'static,
) {
    loop {
        let source = sources.borrow_and_update().clone();
        let Some(mut source) = source else {
            if sources.changed().await.is_err() {
                return;
            }
            continue;
        };
        let initial = source.borrow_and_update().clone();
        if let Some(value) = initial {
            changed(value);
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
                    if let Some(value) = value { changed(value); }
                }
            }
        }
    }
}
