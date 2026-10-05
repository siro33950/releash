# Design

## 変える部分
- ハーネスの依存の持ち方: サーバ用ハーネス（`workflow_control_plane_acceptance`、`agent_session_tui_acceptance`、`workflow_delegate_acceptance`、`client_api_acceptance` のサーバ用部分）が `tauri::App` / `AppHandle` の `manage` / `state` で持っている依存を、ハーネス自身が直接保持する形にし、`tauri::async_runtime::spawn` を使わない。サーバ用ハーネスとサーバ側 lib テストは `desktop_test_support` を参照しない。Tauri に依らない補助（`state_subscriptions` など）は `desktop` の gate の無い場所に置き、シェル用はサーバ用のものを使う（同じ補助を二重に持たない）。根拠: R-001「Tauri のアプリを作らずに、サーバの入口を通して実行される」、B-001。ルート: 委任
- サーバ側 lib テストの依存の組み立て: `desktop_test_support::build_client_dependencies(AppHandle)` 等を使うサーバ側の lib テスト（`adaptor/controller/client/workflow/mod.rs` のテスト、`api/client_test.rs`、`client/app_config/shared_test.rs`、`client/workspace_tree_shared_test.rs`）と `agent_session_tui_acceptance` を、AppHandle なしで依存を組み立てる形にし、`cfg(all(test, feature = "desktop"))` の gate を外す。根拠: R-001、R-002「desktop feature を無効にした構成でもビルド・実行できる」、B-001、B-002。ルート: 委任
- `client_api_acceptance.rs` の分割: サーバ用とシェル用を別のファイルに分ける。シェル用は `desktop` の gate のまま残す。根拠: R-003、B-003。ルート: 委任
- サーバ用ハーネスの Connect クライアント: `connect_client` / `request_client` を、`desktop` の gate の無いもの（`connect_wire::rpc`、`to_rpc` / `to_wire`、生成済みの `client_calls.rs` の include、connectrpc の `HttpClient`）だけで組み立てる。根拠: R-001「Connect の client API を通して」、R-002、B-001、B-002。ルート: 固定（「固定するルート」参照）
- ハーネスとハーネス専用公開の gate: サーバ用ハーネスの module（`src/lib.rs`）と、サーバ内のハーネス専用の公開（domain・usecase・gateway・infrastructure・controller・presenter、`terminal_subscription_acceptance.rs:71,76,81` の getter、`build.rs` が生成コードに埋め込む cfg）から `feature = "desktop"` の条件を外す。根拠: R-002、B-002。ルート: 固定（「固定するルート」参照）
- `TauriApplicationQuitIntentPort` の削除: `adaptor/gateway/application_lifecycle.rs` から削除し、テストの `process_port` には `DaemonProcessActionPort` を渡す。根拠: R-003「Tauri を参照するファイルが [03] で移すものだけ」、B-003。ルート: 固定（「固定するルート」参照）
- Tauri の invoke の拒否の切り出し: `tests/client_api/mod.rs` の invoke 断定をシェルのテストへ移す。根拠: R-005、B-005。ルート: 固定（「固定するルート」参照）
- `ClientConnectionUsecase` の削除: `usecase/client_connection.rs` の `ClientConnectionUsecase`（struct と impl）とその単体テスト、`desktop.rs` と `desktop_client_acceptance.rs` での登録を削除する。サーバ用ハーネスが `ClientConnectionDto` を参照しなくなると、本番で登録されるだけで読まれないコードになるため。根拠: R-003、「固定するルート」の「`ClientConnectionDto` を参照しない」の帰結。ルート: 固定（「変えないもの」のただし書き参照）

## 固定するルート
- ハーネスの公開の gate は `debug_assertions` だけにする（`cfg(any(test, debug_assertions))` 等）。既存の `#[cfg(debug_assertions)] pub mod terminal_subscription_acceptance;`（`src/lib.rs:12-13`）と同じ形にそろえ、新しい Cargo feature は足さない。`desktop` を外すのは、Tauri への参照を抜き終えたサーバ用ハーネスとサーバ内のハーネス専用の公開だけで、シェル用のハーネス（`desktop_test_support`、`client_api_acceptance` のシェル用部分、invoke 拒否のテスト）は `desktop` の gate のまま残す。release ビルドにハーネスが入らない点は変えない。
- サーバ用ハーネスの Connect クライアントは `tests/daemon_smoke.rs:28`、`:240-251` と同じ形で組み立てる。呼び出しの対応は手書きせず、生成済みの `client_calls.rs` を include する。`crate::adaptor::gateway::desktop_client` と `ClientConnectionDto` を参照しない。`desktop_client::client` が付けている設定（既定の timeout など）のうちテストの断定に効くものは、同じ値を proto の service option から読んで付ける（値を直書きしない）。
- `TauriApplicationQuitIntentPort` は移さずに削除する。サーバ側・シェル側どちらのテストも `process_port` には `DaemonProcessActionPort` を渡す。`try_send` の失敗が `ApplicationLifecycleError` になる経路（`application_lifecycle.rs:15-19`）を通るテストがあれば、受け側を保持する。
- B-002 は `cargo test --locked --no-default-features`（`--lib` を付けない全ターゲット）が手元で成功することで確かめる。CI は変えない。シェルのテストとして切り出す invoke 拒否のテストと、分けた後のシェル用ハーネスを使う `tests/` のファイルは、既存のシェルのテストと同じ形の gate（`tests/desktop_daemon.rs:1` の `#![cfg(all(debug_assertions, feature = "desktop"))]`、または `Cargo.toml` の `[[test]]` の `required-features = ["desktop"]`）を持ち、`desktop` 無効の構成でビルドを壊さない。
- Tauri の invoke の拒否は、今と同じ空の command router を持つ mock アプリに対して断定する。シェル用ハーネスにその mock アプリを作る関数を置く。`tests/client_api/mod.rs:73-117` は関数を分け、サーバ側の断定（401/403、Connect 経由の相関と結果）をサーバのテストに残し、invoke の部分（`:101-113`）だけをシェルのテストへ移す。`:146-173` は丸ごとシェルのテストへ移す。断定の式と期待値は変えない。

## 変えないもの
- `adaptor/gateway/desktop_client.rs` と、それを使うシェル側のコード（`daemon_supervision.rs` ほか）。シェルとサーバが共有するクライアント側コードの持ち方は [03]（#1853）で決めるため。 ただし `usecase/client_connection.rs` の `ClientConnectionUsecase`（struct と impl）、その単体テスト、`desktop.rs` と `desktop_client_acceptance.rs` での登録は削除する。本番で登録されるだけで読まれておらず、読んでいたのはサーバ用ハーネスだけだったため。共有の型（`ClientConnectionDto`・`ClientConnectionError`・`ClientConnectionQueryService`）は変えない。
- ハーネスの中の HTTP `/v1` の呼び出し。[05]（#1902）が HTTP の削除と一緒に Connect へ移すため。
- `tauri` の dev-dependency と `desktop` feature、CI のジョブ構成。[03] で扱うため。
- テストの断定の式と期待値。置き換えた後に落ちるテストが出た場合は、期待値を変えずに報告する。

## 未確定・リスク
なし
