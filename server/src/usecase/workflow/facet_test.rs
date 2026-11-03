use crate::usecase::workflow::test_helpers::FakeFacetRepository;
pub(crate) mod tests {
    use super::super::*;
    use super::*;
    use crate::usecase::workflow::test_support::{
        NoopEditorLauncherGateway, NoopEditorSettingsGateway,
    };

    #[test]
    fn duplicate_facet_validates_target_and_copies_source_content() {
        let facets = Arc::new(FakeFacetRepository::default());
        facets
            .save(
                crate::usecase::workflow::test_helpers::facet(
                    FacetKind::Policy,
                    "source",
                    "policy body".into(),
                ),
                true,
            )
            .unwrap();
        let usecase = WorkflowFacetUsecase::new(
            facets.clone(),
            Arc::new(NoopEditorSettingsGateway),
            Arc::new(NoopEditorLauncherGateway),
        );

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
        let usecase = WorkflowFacetUsecase::new(
            facets,
            Arc::new(NoopEditorSettingsGateway),
            Arc::new(NoopEditorLauncherGateway),
        );
        let values = HashMap::from([("request".to_string(), "write tests".to_string())]);

        assert_eq!(
            usecase.render_facet_preview("Task: {{ request }}", &values),
            "Task: write tests"
        );
    }

    #[test]
    fn test_facet保存_builtinを拒否し利用者ファイルは受理する() {
        // Given
        let facets = Arc::new(FakeFacetRepository::default());
        for key in ["builtin", "custom"] {
            facets
                .facets
                .lock()
                .unwrap()
                .insert((FacetKind::Policy, key.into()), "original".into());
        }
        facets
            .builtin_keys
            .lock()
            .unwrap()
            .insert((FacetKind::Policy, "builtin".into()));
        let usecase = WorkflowFacetUsecase::new(
            facets.clone(),
            Arc::new(NoopEditorSettingsGateway),
            Arc::new(NoopEditorLauncherGateway),
        );
        // When
        let result = usecase.save_facet(FacetKind::Policy, "builtin", "changed", false);
        // Then
        assert!(
            matches!(result, Err(WorkflowError::Validation(message)) if message.contains("ビルトイン"))
        );
        assert_eq!(
            facets.get_saved(FacetKind::Policy, "builtin").as_deref(),
            Some("original")
        );
        usecase
            .save_facet(FacetKind::Policy, "custom", "changed", false)
            .unwrap();
        assert_eq!(
            facets.get_saved(FacetKind::Policy, "custom").as_deref(),
            Some("changed")
        );
    }
}
