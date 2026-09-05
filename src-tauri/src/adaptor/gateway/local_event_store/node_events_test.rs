use rusqlite::Connection;

use super::{
    append_node_event, delete_tree, latest_row_for_node_with_event_types, list_tree_roots,
    read_tree, rows_for_event_types, NewNodeEventRow,
};
use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::schema::{initialize_schema, InitialStoreMetadata};

fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    initialize_schema(
        &connection,
        &InitialStoreMetadata {
            installation_id: "00000000-0000-4000-8000-000000000001",
            cursor_hmac_key: &[1; 32],
            operation_binding_hmac_key: &[2; 32],
            process_instance_id: "00000000-0000-4000-8000-000000000002",
            created_at_ms: 1,
        },
        &FaultInjector::new(),
    )
    .unwrap();
    connection
}

fn row(tree_id: &str, node_execution_id: &str, parent_id: Option<&str>) -> NewNodeEventRow {
    NewNodeEventRow {
        tree_id: tree_id.to_string(),
        node_execution_id: node_execution_id.to_string(),
        parent_id: parent_id.map(str::to_string),
        node_name: "main".to_string(),
        kind: "session".to_string(),
        attempt: 1,
        event_type: "started".to_string(),
        session_id: None,
        detail: "{}".to_string(),
    }
}

mod append_node_event_tests {
    use super::*;

    #[test]
    fn test_事実追記_同一treeでseqが単調増加する() {
        // Given: 空の node_events
        let connection = connection();

        // When: 同じ tree に3行 append する
        let first = append_node_event(&connection, &row("tree-1", "root", None), 10).unwrap();
        let second =
            append_node_event(&connection, &row("tree-1", "child", Some("root")), 20).unwrap();
        let third =
            append_node_event(&connection, &row("tree-1", "child", Some("root")), 30).unwrap();

        // Then: seq が 1, 2, 3 と払い出される
        assert_eq!((first, second, third), (1, 2, 3));
    }

    #[test]
    fn test_事実追記_treeごとにseqが独立する() {
        // Given: tree-1 に2行入った状態
        let connection = connection();
        append_node_event(&connection, &row("tree-1", "root", None), 10).unwrap();
        append_node_event(&connection, &row("tree-1", "child", Some("root")), 20).unwrap();

        // When: 別の tree へ append する
        let seq = append_node_event(&connection, &row("tree-2", "root", None), 30).unwrap();

        // Then: tree-2 の seq は 1 から始まる
        assert_eq!(seq, 1);
    }
}

mod read_tree_tests {
    use super::*;

    #[test]
    fn test_tree読み出し_追記順に全行が返る() {
        // Given: tree-1 に3行、tree-2 に1行
        let connection = connection();
        append_node_event(&connection, &row("tree-1", "root", None), 10).unwrap();
        append_node_event(&connection, &row("tree-1", "child", Some("root")), 20).unwrap();
        let mut submit = row("tree-1", "child", Some("root"));
        submit.event_type = "submit_received".to_string();
        submit.detail = "{\"artifact\":\"a-1\"}".to_string();
        append_node_event(&connection, &submit, 30).unwrap();
        append_node_event(&connection, &row("tree-2", "root", None), 40).unwrap();

        // When: tree-1 を読む
        let rows = read_tree(&connection, "tree-1").unwrap();

        // Then: tree-1 の3行だけが seq 順で返り、内容が保存時のまま
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter().map(|row| row.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(rows[2].event_type, "submit_received");
        assert_eq!(rows[2].detail, "{\"artifact\":\"a-1\"}");
        assert_eq!(rows[2].timestamp_ms, 30);
        assert_eq!(rows[1].parent_id.as_deref(), Some("root"));
    }

    #[test]
    fn test_tree読み出し_存在しないtreeは空() {
        // Given: 別 tree のみが存在する
        let connection = connection();
        append_node_event(&connection, &row("tree-1", "root", None), 10).unwrap();

        // When / Then: 未知の tree_id は空集合
        assert!(read_tree(&connection, "missing").unwrap().is_empty());
    }
}

mod latest_row_for_node_with_event_types_tests {
    use super::*;

    #[test]
    fn test_node最新事実取得_対象nodeとevent種別のうち最大seqを返す() {
        // Given: 対象 node の活動・終了・Stopと、対象外の種別・別 node の行
        let connection = connection();
        for (node_execution_id, event_type) in [
            ("session-node", "agent_activity_observed"),
            ("session-node", "process_exited"),
            ("session-node", "stop_received"),
            ("session-node", "submit_received"),
            ("other-node", "agent_activity_observed"),
        ] {
            let mut event = row("tree-1", node_execution_id, None);
            event.event_type = event_type.to_string();
            append_node_event(&connection, &event, 10).unwrap();
        }

        // When: 活動導出に使う3種だけを対象に最新行を読む
        let latest = latest_row_for_node_with_event_types(
            &connection,
            "session-node",
            &["agent_activity_observed", "process_exited", "stop_received"],
        )
        .unwrap()
        .unwrap();

        // Then: 対象3種・対象 node のうち seq 最大のStop行が返る
        assert_eq!(latest.seq, 3);
        assert_eq!(latest.node_execution_id, "session-node");
        assert_eq!(latest.event_type, "stop_received");
    }
}

mod event_type_access_path_tests {
    use super::*;

    #[test]
    fn test_event種別一覧_指定した事実集合だけを全node分まとめてnodeとseq順に返す() {
        // Given: 複数 tree・node の lifecycle 事実と対象外の事実
        let connection = connection();
        for (tree_id, node_execution_id, event_type) in [
            ("tree-b", "node-b", "session_attached"),
            ("tree-a", "node-a", "started"),
            ("tree-a", "node-a", "session_attached"),
            ("tree-a", "node-a", "process_exited"),
            ("tree-b", "node-b", "submit_received"),
            ("tree-b", "node-b", "resume_requested"),
        ] {
            let mut event = row(tree_id, node_execution_id, None);
            event.event_type = event_type.to_string();
            append_node_event(&connection, &event, 10).unwrap();
        }

        // When: lifecycle に必要な event_type 集合を一度に読む
        let rows = rows_for_event_types(
            &connection,
            &["session_attached", "process_exited", "resume_requested"],
        )
        .unwrap();

        // Then: 対象事実だけが node_execution_id・seq の順で返る
        assert_eq!(
            rows.iter()
                .map(|row| {
                    (
                        row.node_execution_id.as_str(),
                        row.seq,
                        row.event_type.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("node-a", 2, "session_attached"),
                ("node-a", 3, "process_exited"),
                ("node-b", 1, "session_attached"),
                ("node-b", 3, "resume_requested"),
            ]
        );
        assert!(rows_for_event_types(&connection, &[]).unwrap().is_empty());
    }
}

mod list_tree_roots_tests {
    use super::*;

    #[test]
    fn test_root一覧_親なしstartedだけが古い順に返る() {
        // Given: 2本の tree と、子 node の started・root の別事実
        let connection = connection();
        append_node_event(&connection, &row("tree-b", "root-b", None), 20).unwrap();
        append_node_event(&connection, &row("tree-a", "root-a", None), 10).unwrap();
        append_node_event(&connection, &row("tree-b", "child", Some("root-b")), 30).unwrap();
        let mut stop = row("tree-b", "root-b", None);
        stop.event_type = "stop_received".to_string();
        append_node_event(&connection, &stop, 40).unwrap();

        // When: root の started を一覧する
        let roots = list_tree_roots(&connection, "started").unwrap();

        // Then: 親なし started の2行が timestamp 昇順で返る
        assert_eq!(
            roots
                .iter()
                .map(|row| row.tree_id.as_str())
                .collect::<Vec<_>>(),
            vec!["tree-a", "tree-b"]
        );
    }
}

mod store_round_trip_tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::store::{
        LocalEventStore, LocalEventStoreConfig,
    };
    use crate::adaptor::gateway::local_event_store::writer::NodeEventWriteError;
    use crate::domain::local_event::LocalEventQueryError;

    #[test]
    fn test_store事実追記_同期文脈で記録され結果が返る() {
        // Given: file-backed store
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();

        // When: store API で2行 append する（1行目は明示時刻・2行目は clock）
        let first = store
            .append_node_event_blocking(row("tree-1", "root", None), Some(1_000))
            .unwrap();
        let second = store
            .append_node_event_blocking(row("tree-1", "child", Some("root")), None)
            .unwrap();

        // Then: seq が直列に払い出され、reader pool から読み出せる
        assert_eq!((first, second), (1, 2));
        let rows = store
            .submit_indexed_query_blocking(|connection| {
                read_tree(connection, "tree-1").map_err(|_| LocalEventQueryError::InvalidRequest)
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].node_execution_id, "root");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_store事実追記_async_runtime上でpanicせず記録され結果が返る() {
        // Given: current-thread tokio runtime 上で利用する file-backed store
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();

        // When: runtime worker 上から同期 append を呼ぶ
        let seq = store
            .append_node_event_blocking(row("tree-async", "root", None), Some(1_000))
            .unwrap();

        // Then: 呼び出しが停止せず結果が返り、事実行を読み出せる
        assert_eq!(seq, 1);
        let rows = store
            .submit_indexed_query_blocking(|connection| {
                read_tree(connection, "tree-async")
                    .map_err(|_| LocalEventQueryError::InvalidRequest)
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].node_execution_id, "root");
    }

    #[test]
    fn test_store事実追記_閉じたwrite_queueはoutcome_unknownを返す() {
        // Given: write queue が閉じた file-backed store
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        store.close_write_queue_for_tests();

        // When: 事実行を追記する
        let error = store
            .append_node_event_blocking(row("tree-closed", "root", None), Some(1_000))
            .unwrap_err();

        // Then: admission の Closed が OutcomeUnknown として返る
        assert_eq!(error, NodeEventWriteError::OutcomeUnknown);
    }

    #[test]
    fn test_store事実追記_reply喪失はoutcome_unknownを返す() {
        // Given: 次の writer reply を失う file-backed store
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        store.fault_injector().arm_drop_reply();

        // When: 事実行を追記する
        let error = store
            .append_node_event_blocking(row("tree-reply-loss", "root", None), Some(1_000))
            .unwrap_err();

        // Then: receiver の切断が OutcomeUnknown として返り、writer は処理済みである
        assert_eq!(error, NodeEventWriteError::OutcomeUnknown);
        let rows = store
            .submit_indexed_query_blocking(|connection| {
                read_tree(connection, "tree-reply-loss")
                    .map_err(|_| LocalEventQueryError::InvalidRequest)
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_store事実追記_writer内のsqlite失敗を返して後続追記を継続する() {
        // Given: node_events.kind の CHECK 制約に違反する行
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let invalid = NewNodeEventRow {
            tree_id: "tree-sqlite-failure".to_string(),
            node_execution_id: "invalid".to_string(),
            parent_id: None,
            node_name: "main".to_string(),
            kind: "invalid-kind".to_string(),
            attempt: 1,
            event_type: "started".to_string(),
            session_id: None,
            detail: "{}".to_string(),
        };

        // When: admission 後の writer thread で INSERT が失敗する
        let error = store
            .append_node_event_blocking(invalid, Some(1_000))
            .unwrap_err();

        // Then: SQLite 失敗が返り、失敗行は記録されていない
        assert_eq!(error, NodeEventWriteError::StorageUnavailable);
        let rows = store
            .submit_indexed_query_blocking(|connection| {
                read_tree(connection, "tree-sqlite-failure")
                    .map_err(|_| LocalEventQueryError::InvalidRequest)
            })
            .unwrap();
        assert!(rows.is_empty());

        // And: writer は停止せず、後続の正常行に seq 1 を払い出す
        let seq = store
            .append_node_event_blocking(row("tree-sqlite-failure", "valid", None), Some(2_000))
            .unwrap();
        assert_eq!(seq, 1);
    }
}

mod delete_tree_tests {
    use super::*;

    #[test]
    fn test_tree削除_対象treeの行だけが物理削除される() {
        // Given: 2本の tree
        let connection = connection();
        append_node_event(&connection, &row("tree-1", "root", None), 10).unwrap();
        append_node_event(&connection, &row("tree-1", "child", Some("root")), 20).unwrap();
        append_node_event(&connection, &row("tree-2", "root", None), 30).unwrap();

        // When: tree-1 を削除する
        let deleted = delete_tree(&connection, "tree-1").unwrap();

        // Then: tree-1 の2行が消え、tree-2 は残る
        assert_eq!(deleted, 2);
        assert!(read_tree(&connection, "tree-1").unwrap().is_empty());
        assert_eq!(read_tree(&connection, "tree-2").unwrap().len(), 1);
    }
}
