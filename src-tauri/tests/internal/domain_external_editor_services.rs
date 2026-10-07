pub(crate) mod tests {

    use releashd::test_support::integration::platform::scan_applications_in;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    pub fn scan_finds_existing_editors_in_dirs() {
        let tmp = TempDir::new().unwrap();
        let apps_dir = tmp.path().to_path_buf();
        fs::create_dir_all(apps_dir.join("Cursor.app")).unwrap();
        fs::create_dir_all(apps_dir.join("Zed.app")).unwrap();

        let editors = scan_applications_in(&[apps_dir]);
        assert_eq!(editors.len(), 2);
        assert_eq!(editors[0].name, "Cursor");
        assert_eq!(editors[1].name, "Zed");
    }

    #[test]
    pub fn scan_deduplicates_across_dirs() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        fs::create_dir_all(tmp1.path().join("Cursor.app")).unwrap();
        fs::create_dir_all(tmp2.path().join("Cursor.app")).unwrap();

        let editors = scan_applications_in(&[tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
        assert_eq!(editors.len(), 1);
        assert!(editors[0].path.contains(tmp1.path().to_str().unwrap()));
    }
}
