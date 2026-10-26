use crate::domain::comment::*;
pub fn agent(backend_id: &str, model: &str) -> Author {
    let _ = model;
    Author::provider_agent(backend_id.to_string(), None)
}
