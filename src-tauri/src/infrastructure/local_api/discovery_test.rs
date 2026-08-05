use super::*;

#[test]
fn test_local_api_discovery_所有fileを非公開権限で作成して削除する() {
    let directory = tempfile::tempdir().unwrap();
    let discovery = LocalApiDiscovery {
        port: 43123,
        token: "secret-token".to_string(),
        instance_id: "instance-1".to_string(),
        pid: 42,
        process_started_at: 123,
    };
    let file = LocalApiDiscoveryFile::create(directory.path(), discovery.clone()).unwrap();

    let decoded: LocalApiDiscovery =
        serde_json::from_slice(&fs::read(file.path()).unwrap()).unwrap();
    assert_eq!(decoded, discovery);
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );

    file.remove_if_owned().unwrap();
    assert!(!file.path().exists());
}

#[test]
fn test_local_api_discovery_古いownerが新しいdiscoveryを削除しない() {
    let directory = tempfile::tempdir().unwrap();
    let stale = LocalApiDiscoveryFile::create(
        directory.path(),
        LocalApiDiscovery {
            port: 40001,
            token: "stale".to_string(),
            instance_id: "instance-stale".to_string(),
            pid: 1,
            process_started_at: 101,
        },
    )
    .unwrap();
    let current = LocalApiDiscoveryFile::create(
        directory.path(),
        LocalApiDiscovery {
            port: 40002,
            token: "current".to_string(),
            instance_id: "instance-current".to_string(),
            pid: 2,
            process_started_at: 202,
        },
    )
    .unwrap();

    stale.remove_if_owned().unwrap();
    assert!(current.path().exists());
    current.remove_if_owned().unwrap();
}
