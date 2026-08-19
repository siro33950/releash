use rusqlite::Connection;

use super::{append_node_event, delete_tree, list_tree_roots, read_tree, NewNodeEventRow};
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
    use crate::domain::local_event::LocalEventQueryError;

    #[tokio::test]
    async fn test_store経由の追記_writerスレッドでseqが払い出され読み出せる() {
        // Given: file-backed store
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();

        // When: store API で2行 append する（1行目は明示時刻・2行目は clock）
        let first = store
            .append_node_event(row("tree-1", "root", None), Some(1_000))
            .await
            .unwrap();
        let second = store
            .append_node_event(row("tree-1", "child", Some("root")), None)
            .await
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

        // When: tree を削除する
        let deleted = store.delete_node_event_tree("tree-1".to_string()).await;

        // Then: 全行が消える
        assert_eq!(deleted.unwrap(), 2);
        let rows = store
            .submit_indexed_query_blocking(|connection| {
                read_tree(connection, "tree-1").map_err(|_| LocalEventQueryError::InvalidRequest)
            })
            .unwrap();
        assert!(rows.is_empty());
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
