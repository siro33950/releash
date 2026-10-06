use super::*;
pub fn agent(backend_id: &str, model: &str) -> ReviewActor {
    let _ = model;
    ReviewActor::provider_agent(backend_id.to_string(), None)
}
