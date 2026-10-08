pub(crate) fn to_canonical_forward_slash(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
#[path = "path_test.rs"]
mod path_tests;
