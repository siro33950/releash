use crate::usecase::workflow::shared_test_helpers::FakeFacetRepository;
pub(crate) mod tests {
    use super::super::*;
    use super::*;

    #[test]
    fn duplicate_facet_validates_target_and_copies_source_content() {
        let facets = Arc::new(FakeFacetRepository::default());
        facets
            .save(FacetKind::Policy, "source", "policy body", true)
            .unwrap();
        let usecase = WorkflowFacetUsecase::new(facets.clone());

        usecase
            .duplicate_facet(FacetKind::Policy, "source", "copy")
            .unwrap();

        assert_eq!(
            facets.get_saved(FacetKind::Policy, "copy").as_deref(),
            Some("policy body")
        );
        assert!(usecase
            .duplicate_facet(FacetKind::Policy, "source", "../bad")
            .is_err());
    }

    #[test]
    fn render_facet_preview_delegates_to_template_preview() {
        let facets = Arc::new(FakeFacetRepository::default());
        let usecase = WorkflowFacetUsecase::new(facets);
        let values = HashMap::from([("request".to_string(), "write tests".to_string())]);

        assert_eq!(
            usecase.render_facet_preview("Task: {{ request }}", &values),
            "Task: write tests"
        );
    }
}
