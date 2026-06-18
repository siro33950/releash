#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyKind {
    Terminal,
    OneShot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_snake_case_wire_value() {
        assert_eq!(
            serde_json::to_string(&PtyKind::Terminal).unwrap(),
            "\"terminal\""
        );
        assert_eq!(
            serde_json::to_string(&PtyKind::OneShot).unwrap(),
            "\"one_shot\""
        );
        assert_eq!(
            serde_json::from_str::<PtyKind>("\"terminal\"").unwrap(),
            PtyKind::Terminal
        );
        assert_eq!(
            serde_json::from_str::<PtyKind>("\"one_shot\"").unwrap(),
            PtyKind::OneShot
        );
    }
}
