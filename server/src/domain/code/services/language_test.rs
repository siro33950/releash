pub(crate) mod language_service_tests {
    use super::super::*;

    #[test]
    fn test_言語判定_typescript拡張子() {
        assert_eq!(get_language_from_path("src/index.ts"), "typescript");
        assert_eq!(get_language_from_path("App.tsx"), "typescript");
    }

    #[test]
    fn test_言語判定_javascript拡張子() {
        assert_eq!(get_language_from_path("index.js"), "javascript");
        assert_eq!(get_language_from_path("Component.jsx"), "javascript");
    }

    #[test]
    fn test_言語判定_rust拡張子() {
        assert_eq!(get_language_from_path("main.rs"), "rust");
    }

    #[test]
    fn test_言語判定_yaml拡張子() {
        assert_eq!(get_language_from_path("config.yaml"), "yaml");
        assert_eq!(get_language_from_path("ci.yml"), "yaml");
    }

    #[test]
    fn test_言語判定_拡張子なし() {
        assert_eq!(get_language_from_path("Makefile"), "plaintext");
    }

    #[test]
    fn test_言語判定_大文字小文字非依存() {
        assert_eq!(get_language_from_path("file.RS"), "rust");
        assert_eq!(get_language_from_path("file.JSON"), "json");
    }

    #[test]
    fn test_言語判定_ドットファイルで拡張子なし() {
        assert_eq!(get_language_from_path(".gitignore"), "plaintext");
    }
}
