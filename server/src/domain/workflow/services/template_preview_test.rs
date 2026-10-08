pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn renders_sample_template_variables_and_keeps_unknown_refs() {
        let values = HashMap::from([("request".to_string(), "write tests".to_string())]);

        assert_eq!(
            render_template_variables("Task: {{ request }} {{ missing }}", &values),
            "Task: write tests {{ missing }}"
        );
    }
}
