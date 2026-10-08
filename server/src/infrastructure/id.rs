pub(crate) fn unique_simple_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
#[path = "id_test.rs"]
mod id_tests;
