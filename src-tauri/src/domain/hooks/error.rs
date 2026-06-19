#[derive(Debug)]
#[allow(dead_code)]
pub enum HooksError {
    Message(String),
}

impl std::fmt::Display for HooksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for HooksError {}
