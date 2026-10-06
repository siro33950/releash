pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn composes_policy_and_user_facet_parts() {
        let contents = FacetContents {
            policy: Some("policy".into()),
            knowledge: vec!["knowledge".into()],
            instruction: Some("instruction".into()),
        };
        let composed = compose_facets(Some(&contents));
        assert_eq!(composed.system_prompt.as_deref(), Some("policy"));
        assert_eq!(composed.user_message, "knowledge\n\ninstruction");
    }

    #[test]
    fn test_provider_tui初期指示_policyとuser_messageを各一度だけ連結する() {
        let instruction =
            provider_tui_initial_instruction(Some("policy"), "knowledge\ninstruction");

        assert_eq!(instruction, "policy\n\nknowledge\ninstruction");
        assert_eq!(instruction.matches("policy").count(), 1);
        assert_eq!(instruction.matches("instruction").count(), 1);
    }
}
