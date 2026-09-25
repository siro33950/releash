use notify_debouncer_mini::{
    new_debouncer,
    notify::{RecommendedWatcher, RecursiveMode},
    Debouncer,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

type Signature = Vec<(String, u64, Option<SystemTime>)>;

fn review_events_signature(dir: &Path) -> std::io::Result<Signature> {
    let mut signature = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(".events.json") {
            continue;
        }
        let metadata = entry.metadata()?;
        signature.push((name.to_string(), metadata.len(), metadata.modified().ok()));
    }
    signature.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(signature)
}

pub(crate) struct ReviewCommentsWatcher {
    _debouncer: Debouncer<RecommendedWatcher>,
    dir: PathBuf,
    signature: Signature,
    error: Arc<Mutex<Option<std::io::Error>>>,
    notify_changed: Arc<dyn Fn() + Send + Sync>,
}

fn io_error(error: notify_debouncer_mini::notify::Error) -> std::io::Error {
    match error.kind {
        notify_debouncer_mini::notify::ErrorKind::Io(error) => error,
        _ => std::io::Error::other(error.to_string()),
    }
}

impl ReviewCommentsWatcher {
    pub(crate) fn start(
        dir: PathBuf,
        notify_changed: Arc<dyn Fn() + Send + Sync>,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let error = Arc::new(Mutex::new(None));
        let errors = error.clone();
        let emit = notify_changed.clone();
        let mut debouncer = new_debouncer(
            Duration::from_millis(500),
            move |result: Result<
                Vec<notify_debouncer_mini::DebouncedEvent>,
                notify_debouncer_mini::notify::Error,
            >| {
                match result {
                    Ok(events) => {
                        if events.iter().any(|event| {
                            event
                                .path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .is_some_and(|name| name.ends_with(".events.json"))
                        }) {
                            emit();
                        }
                    }
                    Err(error) => {
                        *errors.lock().expect("watcher error lock") = Some(io_error(error));
                    }
                }
            },
        )
        .map_err(io_error)?;
        debouncer
            .watcher()
            .watch(&dir, RecursiveMode::NonRecursive)
            .map_err(io_error)?;
        let signature = review_events_signature(&dir)?;
        Ok(Self {
            _debouncer: debouncer,
            dir,
            signature,
            error,
            notify_changed,
        })
    }

    pub(crate) fn poll(&mut self) -> std::io::Result<()> {
        if let Some(error) = self.error.lock().expect("watcher error lock").take() {
            return Err(error);
        }
        let signature = review_events_signature(&self.dir)?;
        if signature != self.signature {
            self.signature = signature;
            (self.notify_changed)();
        }
        Ok(())
    }
}
