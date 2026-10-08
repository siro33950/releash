use super::*;
#[tokio::test]
async fn test_設定適用順序_初回の処理が完了してから後続の処理を実行する() {
    // Given
    let serial = std::sync::Arc::new(Serial::default());
    let calls = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
    let (started, initial_started) = tokio::sync::oneshot::channel();
    let (complete, completion) = tokio::sync::oneshot::channel();
    let initial_serial = serial.clone();
    let initial_calls = calls.clone();
    let initial = tokio::spawn(async move {
        initial_serial
            .call(async {
                started.send(()).unwrap();
                completion.await.unwrap();
                initial_calls.lock().push("initial");
            })
            .await;
    });
    initial_started.await.unwrap();
    let updated_calls = calls.clone();
    let (attempted, update_attempted) = tokio::sync::oneshot::channel();
    let update = tokio::spawn(async move {
        attempted.send(()).unwrap();
        serial
            .call(async {
                updated_calls.lock().push("update");
            })
            .await;
    });
    // When / Then
    update_attempted.await.unwrap();
    assert!(calls.lock().is_empty());
    complete.send(()).unwrap();
    initial.await.unwrap();
    update.await.unwrap();
    assert_eq!(*calls.lock(), ["initial", "update"]);
}
