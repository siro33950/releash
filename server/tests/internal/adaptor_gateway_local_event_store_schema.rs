use releashd::test_support::integration::persistence::table_columns;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;

use releashd::test_support::integration::persistence::StoreLayout;

use releashd::test_support::integration::transport::open_existing_writer;

fn open_store(root: &Path) -> Arc<LocalEventStore> {
    LocalEventStore::open(LocalEventStoreConfig::production(
        root.to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .expect("file-backed local event store")
}

fn database_path(root: &Path) -> PathBuf {
    StoreLayout::new(root).database_path()
}

#[test]
pub fn test_schema_v8_新規storeと再起動で用途を失ったmetadataを持たない() {
    // Given
    let root = tempfile::tempdir().unwrap();
    // When / Then
    for _ in 0..2 {
        drop(open_store(root.path()));
        let connection = open_existing_writer(
            &database_path(root.path()),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        )
        .unwrap();
        assert_eq!(
            table_columns(&connection, "store_metadata").unwrap(),
            [
                "id",
                "schema_version",
                "installation_id",
                "created_at_ms",
                "next_global_sequence",
                "health",
            ]
        );
        releashd::test_support::integration::persistence::validate_current_schema(&connection)
            .unwrap();
    }
}
