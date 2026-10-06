pub(crate) mod tests {
    use std::collections::BTreeMap;

    use opentelemetry::Value;

    use super::super::*;

    #[test]
    fn build_resource_has_exact_attribute_set() {
        let resource = build_resource(config::BuildType::Release);
        let attrs = resource
            .iter()
            .map(|(key, value)| (key.as_str().to_string(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            attrs.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "os.type",
                "releash.build_type",
                "service.name",
                "service.version"
            ]
        );
        assert_eq!(
            attrs.get("releash.build_type"),
            Some(&Value::String("release".into()))
        );
        assert_eq!(
            attrs.get("service.name"),
            Some(&Value::String("releash".into()))
        );
    }

    #[test]
    fn build_resource_uses_build_type_value() {
        let resource = build_resource(config::BuildType::Dev);
        let attrs = resource
            .iter()
            .map(|(key, value)| (key.as_str().to_string(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            attrs.get("releash.build_type"),
            Some(&Value::String("dev".into()))
        );
        assert_eq!(attrs.len(), 4);
    }
}
