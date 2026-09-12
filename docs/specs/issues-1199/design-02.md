# Design 02

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `8f6a107a`。作業ブランチは `feat/issues/1199`。
- 直前の Design は `docs/specs/issues-1199/design-01.md`。design-01 が挙げた変更はすべて作業ツリー上に実装済みであり（`src-tauri/src/adaptor/controller/api/client.rs`、`adaptor/controller/command/client.rs`、`adaptor/protocol/client.rs`、`adaptor/gateway/push.rs`、`infrastructure/push.rs`、`src/lib/clientSocket.ts` ほかの未追跡・変更ファイル群）、Spec 工程ではコードを変更していない。この未コミットの実装を今周の開始状態として扱う。
- この周までに解消となった Thread: `eef79195-8b02-4836-a9b4-7e4ed3a32a8f`（旧 `[server]` を含む既存設定ファイルの扱い）。Requirements の R-011 限定、R-014 / R-015 追加、Behavior の B-015 改訂、B-018 / B-019 / B-020 追加で解決し、開始状態の実装（`adaptor/gateway/app_config/repository_impl.rs` の読み込み・保存経路とそのテスト）がそのまま満たすため、実装変更を伴わない。
- 見送り（`[DEFERRED]`）および不成立（`[REJECTED]`）とした Thread はない。
- 今周の Requirements・Behavior の変更（R-011 の限定、R-014 / R-015、B-015 / B-018 / B-019 / B-020）は開始状態で既に満たされているため、「変える部分」には挙げない。

## 変える部分

- 実通信を伴うテストの配置: 実 git 操作・実 local API 通信・複数レイヤーを通すケースを `src-tauri/tests/` 配下へ移す。`src/` 隣接 module test に残すのは実通信を伴わない serialization / dispatch の検証だけにする。根拠: Thread `9aa0d9fc-556f-43af-980e-ea26bc25754c`。`docs/architecture/TEST.md` の統合テスト配置に対し、`api/client_test.rs`（`git2` で実 repo を作り `LocalApiServerBinding` で実 server を起動し loopback ws へ接続する）と `command/client_test.rs`（実 local API を起動して両 ws route へ接続する）が `src/` 隣接に置かれている。移設後も現在の断定内容（認証拒否、`request_id` 相関、8 イベントの Tauri / ws 一致、UI shell 事象の非経由、接続上限、push 欠落時の切断）が同じ強さで保たれること。ルート: 委任
- `ClientEndpoint` の JSON field 名の断定: 実 serialization 結果から renderer が読む field 名（`url` / `authSubprotocol`）を断定するテストを設ける。根拠: Thread `f4f68b13-310d-488d-b40d-2bd7bc86b6f4`、R-007「renderer は、master token ではないクライアント token を 1 本受け取る」、B-011。`protocol/client.rs` の `rename_all = "camelCase"` を実 serialization から断定するテストがなく、`rename_all` や field 名の変更を検出できない。ルート: 委任
- `listenClient` の初期接続失敗の検証: 初期接続が失敗したとき呼出元へ失敗が伝わること、および後から接続が成功しても失敗した購読の callback が呼ばれないことを断定するテストを設ける。根拠: Thread `1624eeff-d2fe-44d5-9e19-a359af3e407b`、R-009 / B-013、`AGENTS.md`「テスト方針 > フロントエンド」。`src/lib/clientSocket.ts` の初期接続失敗時の listener 解放と再 throw を既存テストのどの経路も通らない。ルート: 委任
- 共有 dispatch 傍受後の既存経路の検証: `ClientCommandDispatch` を state に持つ状態で `CommandRouter::handle` を通し、共有表にない command が従来の domain handler へ到達することを観測可能な結果で断定するテストを設ける。根拠: Thread `f929350e-3de4-4473-ac75-0aacec4c94da`、R-010「ws 経由にした 2 本以外の desktop 機能は、変更前と同じ経路・同じ結果で動作する」、B-014。`command/mod.rs` が全 Tauri invoke を共有 dispatch で傍受する分岐を追加したのに対し、既存テストは `domain_route_index` 単体か共有 command のみを通す。ルート: 委任
- handler 解決の一元化: 表から handler / fallback を選ぶ操作を `CommandRouter` が 1 箇所で所有し、`ClientCommandDispatch` が router の内部構造を直接読まないようにする。根拠: Thread `b11cc564-dfa4-408b-aeda-145176c6f04d`、`docs/architecture/README.md`「同じ操作の実装は 1 つに集約する」。`command/mod.rs` と `command/client.rs` が同じ解決操作を二重に実装し、後者が `CommandRouter` の private field を直接読む。B-007 の断定が変わらないこと。ルート: 委任
- bearer subprotocol 抽出の一元化: `Sec-WebSocket-Protocol` の取得・分割・trim・prefix 選択・handshake での echo を 1 つの実装に集約する。根拠: Thread `923b2b7f-5952-4006-ac1d-8c8f4460e99b`、`docs/architecture/README.md` の同一操作集約。`api/client.rs` と `api/terminal.rs` が同一操作をそれぞれ実装している。client route と `/v1/terminal` の handshake 結果（B-001 / B-002 / B-011）が変わらないこと。ルート: 委任
- クライアント ws の受信サイズ境界: 用途に対して過大な text frame を全量 deserialize の前に拒否する。根拠: Thread `545b9b7f-2d3a-4841-9028-62ad27b26f07`、R-001 / R-002。`api/client.rs` の upgrade が受信上限を設定せず、text を全量 deserialize したうえ失敗時に同じ本文を再解析する。対象は A1 が受理する command 要求の入力境界に限り、Non-goals および Q-006 のとおり A2 の stream 制御の適用は含めない。正常な req/resp（B-003 / B-004）と push 受信（B-005）の結果が変わらないこと。ルート: 委任
- 孤立した直接依存の整理: `src-tauri` の直接依存宣言を実際の直接利用に対応させる。根拠: Thread `dc6d9c46-2256-43cc-ac8e-a4a2599a007e`、R-011。`[server]` 設定の削除で `domain/app_config/services.rs` と `generate_token` 呼出が消え、`rand` の直接利用がなくなった（残る `rand` 名は `gateway/local_event_store/store.rs` の `ring::rand` で別 crate の module）。推移的依存として残る `rand` は対象外。ルート: 委任
- 本番参照を失った定数とテストの削除: 本番の挙動・契約に作用しない定数とテストを残さない。根拠: Thread `f75ba5e2-b4c9-469b-a501-0cc119875c08`、R-006 / B-008。`gateway/repository/notify.rs` の `REPO_PATHS_CHANGED_EVENT` が `cfg(test)` 限定となり、唯一の利用が自身のリテラルとの `assert_eq` になっている。`repo-paths-changed` を含む 8 イベント名が Tauri emit と ws broadcast の双方へ届くことの断定（B-008）は引き続き存在すること。ルート: 委任
- `PushSink` の配線責務の移設: `PushSink` の生成・登録を composition root が所有し、送信経路は配線済みの sink を使うようにする。根拠: Thread `f0d0e0a9-e6b6-40aa-8cd5-f58475e18731`、`docs/architecture/README.md` と `docs/architecture/CONTROLLER.md` の composition root 規約。`infrastructure/push.rs` の `for_app` が未登録時に `app.manage` で生成・登録し、`gateway/push.rs` が全 `BackendPush` 送信でこれを呼ぶため、sink の生成が送信経路の副作用になっている。B-008 / B-009 の結果が変わらないこと。ルート: 委任
- 切断後の ws 購読の回復: 通知欠落等による切断の後も、同じ画面の `workflow-execution-changed` 購読が回復し、backend の現在状態と後続の変化が画面へ反映されるようにする。根拠: Thread `14064710-7ce1-461a-9820-57cc99025283`（blocking）、R-009 / B-013。`infrastructure/push.rs` の broadcast 容量は 64 で `api/client.rs` は Lagged を含む受信エラーで接続を閉じる一方、`src/lib/clientSocket.ts` の fail は connection の破棄と pending の reject だけで listeners の再接続を行わず、`useWorkflowState.ts` の effect は `worktreePath` が変わるまで購読し直さない。R-010 が求める、ws 経由にした 2 本以外の経路・結果を変えないこと。ルート: 委任

## 固定するルート

- 今周に新しく固定する実装上の指定はない。
- design-01 で固定したルートを今周も維持する。local API（`adaptor/controller/api/`、axum）上への ws route 新設、`/v1/terminal` を雛形とするエンベロープ、`Sec-WebSocket-Protocol` bearer 認証、`CommandRequest` / `CommandResponse` と typed push、`attachment_id`＋`sequence` / `ack` と frame 上限・分割の規約、`CommandRouter` の transport 非依存化、push sink の単一化と対象 8 イベント、クライアント token の `TerminalStreamEndpoint` 経路、`[server]` 設定の削除対象、旧 ws shell の非再利用。

## 変えないもの

- ws 経由にした 2 本（`get_current_branch` / `workflow-execution-changed`）以外の desktop 機能の経路と結果。理由: R-010 が変更前と同じ経路・同じ結果での動作を要求しており、今周の 11 件はいずれも既存経路の結果を変えずに解決できるため（design-01 から維持）。
- `menu-event` / `native-file-drop` の既存経路。理由: UI shell の事象であり push sink 単一化の対象外と決まっているため（R-006 / B-009、design-01 から維持）。
- A2 で行う stream フロー制御の送受信時の適用。理由: 受信サイズ境界の対象を A1 が受理する command 要求の入力境界に限ると Thread `545b9b7f-2d3a-4841-9028-62ad27b26f07` の `[FIX_POLICY]` が定めており、Non-goals および Q-006 と一致するため。

## 未確定・リスク

この周までに自動判断した箇所は次のとおり。いずれも人間の確認を経ていない。`[DEFERRED]` で人間へ渡した件はない。

- 自動判断（Q-008、今周に追加）: 既存の設定ファイルに残る `[server]` セクションは、読み込み時に除去せず、次に設定を保存したときに除去する。正本は既存ファイル上のデータの除去時期を定めておらず、起動時の一括書き換えは読み込みだけでは設定ファイルを書き換えないという変更前の挙動を変えるため採らなかった。起動時点での除去が必要と判断された場合、R-015 / B-019 と Non-goals が変わる。
- 自動判断（Q-001〜Q-007、design-01 から継続）: ws 経由にする req/resp 1 本を `get_current_branch`、push 1 本を `workflow-execution-changed` に確定した件、レイテンシ実測値の記録先を `docs/specs/issues-1199/` 配下に確定し許容値（閾値）を定めていない件、ブランチ取得失敗時に Tauri invoke へフォールバックしない件、stream フロー制御と frame 上限・分割を A1 ではエンベロープ定義の規約までとする件、クライアント token を既存の非 master token と同一の 1 本とする件。内容と影響は design-01 の「未確定・リスク」を参照する。
