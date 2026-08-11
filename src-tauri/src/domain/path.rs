pub(crate) fn to_canonical_forward_slash(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_backslashes_to_forward_slashes_only() {
        assert_eq!(to_canonical_forward_slash(r"C:\repo\wt"), "C:/repo/wt");
        assert_eq!(to_canonical_forward_slash("/repo//wt/"), "/repo//wt/");
    }
}
