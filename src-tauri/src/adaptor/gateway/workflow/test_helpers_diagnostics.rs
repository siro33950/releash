pub const PREDICATE_ROUTING: &str = include_str!("fixtures/valid/predicate-routing.yml");
pub const NESTED_PREDICATE: &str = "{and: [passed, {or: [clean, skipped]}]}";
pub fn predicate_yaml(on: &str) -> String {
    PREDICATE_ROUTING.replace(NESTED_PREDICATE, on)
}
