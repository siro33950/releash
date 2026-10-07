use super::*;

#[test]
fn default_data_dir_name_distinguishes_dev_and_production() {
    assert_eq!(
        default_data_dir_name_for_profile(BuildProfile::Production),
        "com.releash.app"
    );
    assert_eq!(
        default_data_dir_name_for_profile(BuildProfile::Development),
        "com.releash.app.dev"
    );
}
