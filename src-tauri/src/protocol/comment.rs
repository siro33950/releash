use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentAuthor {
    #[serde(rename = "type")]
    pub author_type: String,
    pub name: String,
}

impl Default for CommentAuthor {
    fn default() -> Self {
        Self {
            author_type: "human".to_string(),
            name: "User".to_string(),
        }
    }
}

fn default_author() -> CommentAuthor {
    CommentAuthor::default()
}

fn default_target() -> String {
    "local".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddComment {
    pub file_path: String,
    pub line_number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub content: String,
    #[serde(default = "default_author")]
    pub author: CommentAuthor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default = "default_target")]
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentItem {
    pub id: String,
    pub file_path: String,
    pub line_number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub content: String,
    pub status: String,
    pub created_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default = "default_author")]
    pub author: CommentAuthor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default = "default_target")]
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteComment {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateComment {
    pub id: String,
    pub content: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveComment {
    pub id: String,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentSync {
    pub comments: Vec<CommentItem>,
}
