use crate::domain::workflow::{NodeFact, SessionAttachedFact};

pub fn session_attached(session_id: &str) -> NodeFact {
    NodeFact::SessionAttached(SessionAttachedFact {
        session_id: session_id.to_string(),
        provider_session_id: None,
        transcript_ref: None,
        initial_instruction_admitted: false,
    })
}
