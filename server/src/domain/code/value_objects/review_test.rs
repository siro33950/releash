pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn review_blob_content_type_classifies_images_without_mime() {
        let png = ReviewBlobContentType::from_path("assets/LOGO.PNG");
        assert!(png.is_image());

        let svg = ReviewBlobContentType::image_from_path("icons/app.svg").unwrap();
        assert!(svg.is_image());

        let binary = ReviewBlobContentType::from_path("archive.bin");
        assert!(!binary.is_image());
        assert_eq!(ReviewBlobContentType::image_from_path("archive.bin"), None);
    }
}

#[test]
fn test_表示分類_欠損と非utf8とnulを区別する() {
    use crate::domain::code::ReviewSideBytes;
    // Given / When / Then
    for (bytes, binary) in [
        (ReviewSideBytes::Missing, false),
        (ReviewSideBytes::Present(b"text".to_vec()), false),
        (ReviewSideBytes::Present(vec![0]), true),
        (ReviewSideBytes::Present(vec![255]), true),
    ] {
        assert_eq!(super::review_bytes_are_binary(&bytes), binary);
    }
}

#[test]
fn test_表示分類_画像と属性でのbinaryをdomainが決める() {
    use super::ReviewPresentation as Presentation;
    use crate::domain::code::ReviewSideBytes;
    // Given / When / Then
    let missing = ReviewSideBytes::Missing;
    assert_eq!(
        Presentation::classify("file.png", false, &missing, &missing),
        Presentation::Image
    );
    assert_eq!(
        Presentation::classify("file.rs", true, &missing, &missing),
        Presentation::Binary
    );
    assert_eq!(
        Presentation::classify("file.rs", false, &missing, &missing),
        Presentation::Text
    );
}

#[test]
fn test_初期表示_選択した比較元との一致でfileとdiffを決める() {
    // Given / When / Then
    assert_eq!(
        super::ReviewPresentation::initial_file_mode(true),
        super::FileMode::File
    );
    assert_eq!(
        super::ReviewPresentation::initial_file_mode(false),
        super::FileMode::Diff
    );
}

#[test]
fn test_表示選択_既知の値を型に変換し未知の値を拒否する() {
    // Given / When / Then
    for mode in [super::FileMode::File, super::FileMode::Diff] {
        assert_eq!(super::FileMode::parse(mode.as_str()).unwrap(), mode);
    }
    assert!(super::FileMode::parse("unknown").is_err());
}

#[test]
fn test_属性バイナリの初期表示_比較の変更有無を表示選択にする() {
    // Given / When / Then
    assert_eq!(super::FileMode::for_change(false), super::FileMode::File);
    assert_eq!(super::FileMode::for_change(true), super::FileMode::Diff);
}
