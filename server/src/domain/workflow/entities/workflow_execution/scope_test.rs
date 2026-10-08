pub(crate) mod tests {
    use super::super::*;

    fn sequence_scope() -> SequenceScopeRuntime {
        SequenceScopeRuntime::default()
    }

    #[test]
    fn record_child_start_counts_from_one_and_increases_monotonically() {
        let mut scope = sequence_scope();

        assert_eq!(scope.record_child_start("fix"), 1);
        assert_eq!(scope.record_child_start("fix"), 2);
        assert_eq!(scope.record_child_start("exit"), 1);
        assert_eq!(scope.record_child_start("fix"), 3);
    }

    #[test]
    fn raise_child_count_to_never_lowers_the_count() {
        let mut scope = sequence_scope();
        scope.raise_child_count_to("fix", 3);
        assert_eq!(scope.record_child_start("fix"), 4);

        scope.raise_child_count_to("fix", 2);
        assert_eq!(scope.record_child_start("fix"), 5);
    }
}
