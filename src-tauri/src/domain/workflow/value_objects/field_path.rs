#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldPath {
    segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldPathError {
    EmptyReference,
    InvalidSegment { position: usize, value: String },
}

impl FieldPath {
    pub fn new<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            segments: segments.into_iter().map(Into::into).collect(),
        }
    }

    pub fn from_reference(reference: &str) -> Result<(String, Self), FieldPathError> {
        if reference.is_empty() {
            return Err(FieldPathError::EmptyReference);
        }
        let parts = reference.split('.').collect::<Vec<_>>();
        if let Some((position, value)) = parts
            .iter()
            .enumerate()
            .find(|(_, segment)| !is_valid_segment(segment))
        {
            return Err(FieldPathError::InvalidSegment {
                position,
                value: (*value).to_string(),
            });
        }
        let root = parts[0].to_string();
        let field_path = Self::new(parts.into_iter().skip(1));
        Ok((root, field_path))
    }

    pub fn from_dotted(value: &str) -> Result<Self, FieldPathError> {
        if value.is_empty() {
            return Err(FieldPathError::EmptyReference);
        }
        let segments = value.split('.').collect::<Vec<_>>();
        if let Some((position, value)) = segments
            .iter()
            .enumerate()
            .find(|(_, segment)| segment.is_empty())
        {
            return Err(FieldPathError::InvalidSegment {
                position,
                value: (*value).to_string(),
            });
        }
        Ok(Self::new(segments))
    }

    pub fn to_reference(&self, root: &str) -> Result<String, FieldPathError> {
        if !is_valid_segment(root) {
            return Err(FieldPathError::InvalidSegment {
                position: 0,
                value: root.to_string(),
            });
        }
        if let Some((position, value)) = self
            .segments
            .iter()
            .enumerate()
            .find(|(_, segment)| !is_valid_segment(segment))
        {
            return Err(FieldPathError::InvalidSegment {
                position: position + 1,
                value: value.clone(),
            });
        }
        if self.segments.is_empty() {
            return Ok(root.to_string());
        }
        Ok(format!("{root}.{}", self.segments.join(".")))
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    pub fn as_string(&self) -> String {
        self.segments.join(".")
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

impl std::fmt::Display for FieldPath {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.as_string())
    }
}

pub(crate) fn is_valid_segment(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

#[cfg(test)]
#[path = "field_path_test.rs"]
mod field_path_test;
