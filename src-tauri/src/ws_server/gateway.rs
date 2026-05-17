use std::sync::Arc;

use crate::backends::ModelInfo;
use crate::protocol::{BackendModelsUpdated, ModelInfoMsg, WsMessage};
use crate::usecase::backend_models::BackendModelsUpdateNotifier;
use crate::ws_bridge::WsBroadcaster;

pub(crate) struct WsBackendModelsUpdateNotifier {
    broadcaster: Arc<WsBroadcaster>,
}

impl WsBackendModelsUpdateNotifier {
    pub(crate) fn new(broadcaster: Arc<WsBroadcaster>) -> Self {
        Self { broadcaster }
    }
}

pub(crate) fn build_backend_models_updated_message(
    backend_id: &str,
    available_models: &[ModelInfo],
) -> WsMessage {
    WsMessage::BackendModelsUpdated(BackendModelsUpdated {
        backend_id: backend_id.to_string(),
        available_models: available_models.iter().map(ModelInfoMsg::from).collect(),
    })
}

pub(crate) fn broadcast_backend_models_updated(
    broadcaster: &WsBroadcaster,
    backend_id: &str,
    available_models: &[ModelInfo],
) {
    broadcaster.send_without_buffer(build_backend_models_updated_message(
        backend_id,
        available_models,
    ));
}

impl BackendModelsUpdateNotifier for WsBackendModelsUpdateNotifier {
    fn broadcast_backend_models_updated(&self, backend_id: &str, available_models: &[String]) {
        let available_models: Vec<ModelInfo> = available_models
            .iter()
            .map(|value| ModelInfo {
                value: value.clone(),
            })
            .collect();
        broadcast_backend_models_updated(&self.broadcaster, backend_id, &available_models);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_backend_models_updated_message_uses_ws_contract_fields() {
        let msg = build_backend_models_updated_message(
            "codex",
            &[ModelInfo {
                value: "gpt-5.5".to_string(),
            }],
        );
        match msg {
            WsMessage::BackendModelsUpdated(payload) => {
                assert_eq!(payload.backend_id, "codex");
                assert_eq!(payload.available_models.len(), 1);
                assert_eq!(payload.available_models[0].value, "gpt-5.5");
            }
            _ => panic!("expected BackendModelsUpdated"),
        }
    }
}
