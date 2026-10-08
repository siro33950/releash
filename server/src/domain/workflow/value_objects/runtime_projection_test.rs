pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn token_usage_adds_input_and_output() {
        let mut usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
        };
        usage.add(&TokenUsage {
            input_tokens: 3,
            output_tokens: 4,
        });
        assert_eq!(usage.input_tokens, 4);
        assert_eq!(usage.output_tokens, 6);
    }
}
