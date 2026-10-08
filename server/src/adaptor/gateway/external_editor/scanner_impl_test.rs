pub(crate) mod tests {
    use super::super::*;
    use std::path::Path;

    #[test]
    fn application_dirs_includes_system() {
        let dirs = application_dirs();
        assert!(dirs.iter().any(|d| d == Path::new("/Applications")));
    }
}
