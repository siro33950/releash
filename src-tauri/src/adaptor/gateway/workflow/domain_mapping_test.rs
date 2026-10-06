pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn workflow_definition_to_domain_preserves_facet_refs_without_runtime_contents() {
        let workflow = schema::WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![schema::NodeDefinition {
                name: "implement".to_string(),
                kind: schema::NodeKind::Session(schema::SessionSpec {
                    facets: schema::FacetRefs {
                        knowledge: vec!["knowledge-a".to_string(), "knowledge-b".to_string()],
                        instruction: Some("inst".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            entry: "implement".to_string(),
        };

        let mapped = workflow_definition_to_domain(&workflow);

        assert_eq!(
            mapped.nodes[0].session().unwrap().facets.knowledge,
            vec!["knowledge-a", "knowledge-b"]
        );
        assert_eq!(
            mapped.nodes[0]
                .session()
                .unwrap()
                .facets
                .instruction
                .as_deref(),
            Some("inst")
        );
    }

    #[test]
    fn workflow_definition_to_domain_preserves_fanout_child_and_items() {
        let workflow = schema::WorkflowDefinitionYaml {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![schema::NodeDefinition {
                name: "main".to_string(),
                kind: schema::NodeKind::Fanout(schema::FanoutSpec {
                    children: vec![
                        domain::ChildEntry::reference("lint"),
                        domain::ChildEntry::reference("test"),
                    ],
                    items: Some(schema::ItemsSource::ArtifactField {
                        node: "plan".to_string(),
                        field_path: domain::FieldPath::new(["targets"]),
                    }),
                }),
                ..Default::default()
            }],
            entry: "main".to_string(),
        };

        let mapped = workflow_definition_to_domain(&workflow);
        let fanout = mapped.nodes[0].fanout().unwrap();

        assert_eq!(
            fanout.children,
            vec![
                domain::ChildEntry::reference("lint"),
                domain::ChildEntry::reference("test"),
            ]
        );
        assert_eq!(
            fanout.items,
            Some(domain::ItemsSource::ArtifactField {
                node: "plan".to_string(),
                field_path: domain::FieldPath::new(["targets"]),
            })
        );
    }
}
