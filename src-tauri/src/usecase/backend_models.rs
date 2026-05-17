use std::sync::Arc;

pub(crate) trait BackendModelsUpdateNotifier: Send + Sync {
    fn broadcast_backend_models_updated(&self, backend_id: &str, available_models: &[String]);
}

pub(crate) type BackendModelsUpdateNotifierState = Arc<dyn BackendModelsUpdateNotifier>;
