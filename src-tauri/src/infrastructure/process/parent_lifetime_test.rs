use super::*;

#[test]
fn test_親終了回収_spawnが完了するまで排他回収を開始しない() {
    // Given
    let spawning = spawn_guard();
    let (attempted, attempt) = std::sync::mpsc::channel();
    let (acquired, acquisition) = std::sync::mpsc::channel();
    let cleanup = std::thread::spawn(move || {
        attempted.send(()).unwrap();
        let _cleanup = CHILD_SPAWNS.write();
        acquired.send(()).unwrap();
    });
    attempt.recv().unwrap();
    // When / Then
    assert!(acquisition.try_recv().is_err());
    // When
    drop(spawning);
    // Then
    acquisition
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    cleanup.join().unwrap();
}
