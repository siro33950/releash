use std::collections::HashMap;

pub(crate) struct DesktopChannel<T: serde::Serialize + Clone> {
    channels: parking_lot::Mutex<HashMap<String, tauri::ipc::Channel<T>>>,
}

impl<T: serde::Serialize + Clone> DesktopChannel<T> {
    pub(crate) fn new() -> Self {
        Self {
            channels: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn register(&self, id: String, channel: tauri::ipc::Channel<T>) {
        self.channels.lock().insert(id, channel);
    }

    pub(crate) fn stop(&self, id: &str) {
        self.channels.lock().remove(id);
    }

    pub(crate) fn send(&self, id: &str, value: T) {
        let mut channels = self.channels.lock();
        if channels
            .get(id)
            .is_some_and(|channel| channel.send(value).is_err())
        {
            channels.remove(id);
        }
    }

    pub(crate) fn publish(&self, value: T) {
        self.channels
            .lock()
            .retain(|_, channel| channel.send(value.clone()).is_ok());
    }
}

#[cfg(test)]
#[path = "desktop_channel_test.rs"]
mod desktop_channel_tests;
