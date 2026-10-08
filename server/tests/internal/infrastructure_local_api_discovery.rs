#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use releashd::test_support::integration::transport::lookup_process_start_time;
use releashd::test_support::integration::transport::process_start_time;
use releashd::test_support::integration::transport::LocalApiDiscovery;
use releashd::test_support::integration::transport::LocalApiDiscoveryFile;
use std::fs;

#[test]
pub fn test_process開始時刻参照_現在processの生値を返す() {
    // Given
    let pid = std::process::id();

    // When
    let result = lookup_process_start_time(pid);

    // Then
    assert!(result.process_list_available);
    assert!(result.start_time.is_some_and(|start_time| start_time != 0));
    assert_eq!(process_start_time(pid), result.start_time);
}

#[test]
pub fn test_process開始時刻参照_対象不在でもprocess一覧の参照結果を返す() {
    // Given / When
    let result = lookup_process_start_time(u32::MAX);

    // Then
    assert!(result.process_list_available);
    assert_eq!(result.start_time, None);
}

#[test]
pub fn test_local_api_discovery_所有fileを非公開権限で作成して削除する() {
    let directory = tempfile::tempdir().unwrap();
    let discovery = LocalApiDiscovery {
        port: 43123,
        token: "secret-token".to_string(),
        daemon_id: "instance-1".to_string(),
        pid: 42,
        process_started_at: 123,
        ..Default::default()
    };
    let file = LocalApiDiscoveryFile::create_client(directory.path(), discovery.clone()).unwrap();

    let decoded: serde_json::Value =
        serde_json::from_slice(&fs::read(file.path()).unwrap()).unwrap();
    assert_eq!(decoded, serde_json::to_value(discovery).unwrap());
    assert_eq!(decoded["daemon_id"], "instance-1");
    assert!(decoded.get("instance_id").is_none());
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );

    file.remove_if_owned().unwrap();
    assert!(!file.path().exists());
}

#[test]
pub fn test_local_api_discovery_古いownerが新しいdiscoveryを削除しない() {
    let directory = tempfile::tempdir().unwrap();
    let stale = LocalApiDiscoveryFile::create_client(
        directory.path(),
        LocalApiDiscovery {
            port: 40001,
            token: "stale".to_string(),
            daemon_id: "instance-stale".to_string(),
            pid: 1,
            process_started_at: 101,
            ..Default::default()
        },
    )
    .unwrap();
    let current = LocalApiDiscoveryFile::create_client(
        directory.path(),
        LocalApiDiscovery {
            port: 40002,
            token: "current".to_string(),
            daemon_id: "instance-current".to_string(),
            pid: 2,
            process_started_at: 202,
            ..Default::default()
        },
    )
    .unwrap();

    stale.remove_if_owned().unwrap();
    assert!(current.path().exists());
    current.remove_if_owned().unwrap();
}

#[test]
pub fn test_local_api_discovery_破損したfileを削除する() {
    for content in [&b"invalid json"[..], b"[]"] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let file =
            LocalApiDiscoveryFile::create_client(directory.path(), LocalApiDiscovery::default())
                .unwrap();
        fs::write(file.path(), content).unwrap();

        // When
        file.remove_if_owned().unwrap();

        // Then
        assert!(!file.path().exists());
    }
}
