pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn native_file_drop_serializes_correctly() {
        let drop = NativeFileDrop {
            paths: vec!["/Users/test/file.txt".to_string()],
            position: (100.0, 200.0),
        };
        let json = serde_json::to_value(&drop).unwrap();
        assert_eq!(json["paths"][0], "/Users/test/file.txt");
        assert_eq!(json["position"][0], 100.0);
        assert_eq!(json["position"][1], 200.0);
    }

    #[test]
    fn native_file_drop_multiple_paths() {
        let drop = NativeFileDrop {
            paths: vec![
                "/Users/test/a.txt".to_string(),
                "/Users/test/b.txt".to_string(),
            ],
            position: (50.0, 75.0),
        };
        let json = serde_json::to_value(&drop).unwrap();
        assert_eq!(json["paths"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn cocoa_to_css_y_flip() {
        // Cocoa (200, 600) → CSS (200, 200) for frame_height=800
        let (x, y) = convert_cocoa_to_css(200.0, 600.0, 800.0);
        assert_eq!(x, 200.0);
        assert_eq!(y, 200.0);
    }

    #[test]
    fn cocoa_to_css_origin() {
        // 左下原点 (0, 0) → CSS左上原点 (0, frame_height)
        let (x, y) = convert_cocoa_to_css(0.0, 0.0, 1000.0);
        assert_eq!(x, 0.0);
        assert_eq!(y, 1000.0);
    }
}
