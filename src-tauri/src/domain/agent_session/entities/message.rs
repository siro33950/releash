use super::message_part::MessagePart;

#[allow(dead_code)]
// issues-1301 G-1: domain entity retained for backend-boundary migration while legacy session DTO projection is still being collapsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub thinking: Option<String>,
    pub parts: Vec<MessagePart>,
    pub timestamp: f64,
}

#[allow(dead_code)]
// issues-1301 G-1: domain entity retained for backend-boundary migration while legacy session DTO projection is still being collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    Human,
    Agent,
    System,
}
