use super::*;
fn discovery() -> LocalApiDiscovery {
    LocalApiDiscovery {
        port: 1234,
        token: "operator".into(),
        instance_id: "daemon".into(),
        pid: 42,
        process_started_at: 100,
    }
}
#[test]
fn test_発見ファイル_プロセス不在と再利用を拒否する() {
    let file = discovery();
    for time in [None, Some(101)] {
        assert_eq!(
            file.verify_process(|_| ProcessStartTimeLookup {
                process_list_available: true,
                start_time: time
            })
            .unwrap_err()
            .code,
            connectrpc::ErrorCode::Unavailable
        );
    }
    file.verify_process(|_| ProcessStartTimeLookup {
        process_list_available: true,
        start_time: Some(100),
    })
    .unwrap();
}
#[test]
fn test_発見ファイル_サーバの同一性を確認する() {
    let file = discovery();
    let mut info = ServerInfo {
        daemon_id: file.instance_id.clone(),
        pid: file.pid,
        process_started_at: file.process_started_at,
        ..Default::default()
    };
    file.verify_server(&info).unwrap();
    info.daemon_id = "other".into();
    assert_eq!(
        file.verify_server(&info).unwrap_err().code,
        connectrpc::ErrorCode::Unavailable
    );
}

#[test]
fn test_発見ファイル_欠損と破損をunavailableにする() {
    let directory = tempfile::tempdir().unwrap();
    assert!(read_optional(directory.path()).unwrap().is_none());
    assert_eq!(
        read(directory.path()).unwrap_err().code,
        connectrpc::ErrorCode::Unavailable
    );
    std::fs::write(directory.path().join("client-api.json"), b"invalid json").unwrap();
    assert!(matches!(
        read_optional(directory.path()),
        Err(DiscoveryReadError::Decode(_))
    ));
    assert_eq!(
        read(directory.path()).unwrap_err().code,
        connectrpc::ErrorCode::Unavailable
    );
    std::fs::write(
        directory.path().join("client-api.json"),
        serde_json::to_vec(&discovery()).unwrap(),
    )
    .unwrap();
    assert_eq!(read(directory.path()).unwrap(), discovery());
}

#[test]
fn test_発見ファイル_不正なmetadataと取得不能なprocess情報を拒否する() {
    for field in ["port", "token", "instance_id", "pid", "process_started_at"] {
        let mut value = serde_json::to_value(discovery()).unwrap();
        value[field] = if field == "token" || field == "instance_id" {
            serde_json::json!(" ")
        } else {
            serde_json::json!(0)
        };
        let file: LocalApiDiscovery = serde_json::from_value(value).unwrap();
        let error = file
            .verify_process(|_| panic!("invalid metadata must not query processes"))
            .unwrap_err();
        assert_eq!(error.code, connectrpc::ErrorCode::Unavailable);
    }
    assert_eq!(
        discovery()
            .verify_process(|_| ProcessStartTimeLookup {
                process_list_available: false,
                start_time: None
            })
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::Unavailable
    );
}
