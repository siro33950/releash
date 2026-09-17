use crate::usecase::desktop_update::{DesktopUpdateGateway, UpdateInfo};
use std::sync::Arc;
use tauri::Emitter;
use tauri_plugin_updater::UpdaterExt;

type DownloadedUpdate = (Arc<dyn UpdatePackage>, Vec<u8>);
type Progress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;
#[async_trait::async_trait]
trait UpdatePackage: Send + Sync {
    fn info(&self) -> UpdateInfo;
    async fn download(&self, progress: Progress) -> Result<Vec<u8>, String>;
    fn install(&self, bytes: Vec<u8>) -> Result<(), String>;
}
#[async_trait::async_trait]
trait UpdateRuntime: Send + Sync {
    async fn check(&self) -> Result<Option<Arc<dyn UpdatePackage>>, String>;
    fn progress(&self, percent: u32);
    fn restart(&self) -> Result<(), String>;
}
struct TauriRuntime(tauri::AppHandle);
#[async_trait::async_trait]
impl UpdateRuntime for TauriRuntime {
    async fn check(&self) -> Result<Option<Arc<dyn UpdatePackage>>, String> {
        self.0
            .updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map(|update| update.map(|update| Arc::new(update) as Arc<dyn UpdatePackage>))
            .map_err(|e| e.to_string())
    }
    fn progress(&self, percent: u32) {
        let _ = self.0.emit("desktop-update-progress", percent);
    }
    fn restart(&self) -> Result<(), String> {
        crate::infrastructure::platform::desktop_restart::restart(&self.0)
    }
}
#[async_trait::async_trait]
impl UpdatePackage for tauri_plugin_updater::Update {
    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            version: self.version.clone(),
            notes: self.body.clone().unwrap_or_default(),
        }
    }
    async fn download(&self, progress: Progress) -> Result<Vec<u8>, String> {
        self.download(move |chunk, total| progress(chunk as u64, total), || {})
            .await
            .map_err(|e| e.to_string())
    }
    fn install(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.install(bytes).map_err(|e| e.to_string())
    }
}

pub(crate) struct TauriUpdateGateway {
    runtime: Arc<dyn UpdateRuntime>,
    update: parking_lot::Mutex<Option<Arc<dyn UpdatePackage>>>,
    downloaded: parking_lot::Mutex<Option<DownloadedUpdate>>,
}
impl TauriUpdateGateway {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self::with_runtime(Arc::new(TauriRuntime(app)))
    }
    fn with_runtime(runtime: Arc<dyn UpdateRuntime>) -> Self {
        Self {
            runtime,
            update: parking_lot::Mutex::new(None),
            downloaded: parking_lot::Mutex::new(None),
        }
    }
}
#[async_trait::async_trait]
impl DesktopUpdateGateway for TauriUpdateGateway {
    async fn check(&self) -> Result<Option<UpdateInfo>, String> {
        let update = self.runtime.check().await?;
        let info = update.as_ref().map(|update| update.info());
        *self.update.lock() = update;
        Ok(info)
    }
}
#[async_trait::async_trait]
impl crate::domain::daemon_supervision::DesktopUpdateInstaller for TauriUpdateGateway {
    async fn download(&self) -> Result<(), String> {
        let update = self
            .update
            .lock()
            .clone()
            .ok_or("No update is available.")?;
        let downloaded = std::sync::atomic::AtomicU64::new(0);
        let runtime = self.runtime.clone();
        let bytes = update
            .download(Arc::new(move |chunk, total| {
                let bytes = downloaded
                    .fetch_add(chunk, std::sync::atomic::Ordering::Relaxed)
                    .saturating_add(chunk);
                if let Some(total) = total.filter(|size| *size != 0) {
                    runtime.progress(
                            ((bytes as f64 / total as f64) * 100.0).round().min(100.0) as u32,
                        );
                }
            }))
            .await?;
        *self.downloaded.lock() = Some((update, bytes));
        Ok(())
    }
    async fn install(&self) -> Result<(), String> {
        let (update, bytes) = self
            .downloaded
            .lock()
            .take()
            .ok_or("No verified update has been downloaded.")?;
        tokio::task::spawn_blocking(move || update.install(bytes))
            .await
            .map_err(|e| e.to_string())?
    }
    fn restart(&self) -> Result<(), String> {
        self.runtime.restart()
    }
}

#[cfg(test)]
#[path = "desktop_update_test.rs"]
mod desktop_update_tests;
