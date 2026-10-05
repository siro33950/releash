use super::*;

#[test]
fn test_購読停止_状態変化を待たず登録を解放する() {
    // Given
    let channels = DesktopChannel::<String>::new();
    let (sender, receiver) = std::sync::mpsc::channel();
    let channel = tauri::ipc::Channel::new(move |message| {
        sender.send(message).unwrap();
        Ok(())
    });
    channels.register("screen".into(), channel);
    // When
    channels.send("screen", "starting".into());
    channels.publish("restoring".into());
    channels.stop("screen");
    channels.publish("ready".into());
    // Then
    assert!(channels.channels.lock().is_empty());
    assert_eq!(receiver.try_iter().count(), 2);
}
