use super::*;

#[test]
fn stable_file_identity_comparison_detects_only_known_mismatches() {
    let first = DatabaseFileIdentity {
        stable: Some(StableFileId {
            volume: 1,
            index: 2,
        }),
    };
    let same = DatabaseFileIdentity {
        stable: Some(StableFileId {
            volume: 1,
            index: 2,
        }),
    };
    let replaced = DatabaseFileIdentity {
        stable: Some(StableFileId {
            volume: 1,
            index: 3,
        }),
    };
    let unavailable = DatabaseFileIdentity { stable: None };

    assert!(!database_identity_changed(first, same));
    assert!(database_identity_changed(first, replaced));
    assert!(!database_identity_changed(first, unavailable));
    assert!(!database_identity_changed(unavailable, replaced));
}
