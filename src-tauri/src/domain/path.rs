pub(crate) fn to_canonical_forward_slash(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_backslashes_to_forward_slashes_only() {
        assert_eq!(to_canonical_forward_slash(r"C:\repo\wt"), "C:/repo/wt");
        assert_eq!(
            to_canonical_forward_slash(r"\\server\share\\wt"),
            "//server/share//wt"
        );
        assert_eq!(to_canonical_forward_slash("/repo//wt/"), "/repo//wt/");
    }

    #[test]
    fn is_idempotent_after_conversion() {
        let once = to_canonical_forward_slash(r"C:\repo\wt");
        let twice = to_canonical_forward_slash(&once);

        assert_eq!(twice, once);
    }
}
