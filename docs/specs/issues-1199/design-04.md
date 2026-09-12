# Design 04

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `8f6a107a`。作業ブランチは `feat/issues/1199`。
- 直前の Design は `docs/specs/issues-1199/design-03.md`。design-03 が挙げた 7 件のうち 6 件は作業ツリー上に実装済みであり、Spec 工程ではコードを変更していないため、この未コミットの実装を今周の開始状態として扱う。実装済みの内訳は次のとおり。実通信を伴うテストは `src-tauri/Cargo.toml:94-96` の `[[test]] name = "client_api"` によって統合テスト target として扱われ、`src-tauri/src/lib.rs` からは取り込まれていない。`src-tauri/src/adaptor/controller/api/client.rs:10` は `infrastructure::push` ではなく `adaptor::gateway::push::{ClientPushGateway, ClientPushSubscription}` に依存する。`ClientRequest` / `CommandRequest` / `CommandResponse` / `CommandOutcome` / `StreamEnvelope` は `api/protocol.rs:158-248` にあり、`adaptor/protocol/client.rs` に残るのは `ClientEndpoint` と `CLIENT_WS_PATH` だけである。`ClientEndpoint` の公開項目は `url` と `auth_subprotocol` の 2 つで、frame 上限値の項目はない。`api/client.rs:66-102` は dispatch の完了待ちを `stream.then` の中に保持したまま `select` で push の受信を進める。`src/lib/clientSocket.ts:130-146` は購読を先に登録してから接続を試み、失敗しても購読を保持して再接続を予約し、接続確立時に `onReconnect` を呼ぶ。
- 残る 1 件（レイテンシ記録から計測テストへの参照の更新）は未達である。`docs/specs/issues-1199/latency.md:38` が参照する `../../../src-tauri/src/adaptor/controller/api/client_test.rs` は存在せず（同ディレクトリの実一覧は `auth.rs` / `client.rs` / `error.rs` / `mod.rs` / `protocol.rs` / `protocol_test.rs` / `provider_lifecycle.rs` / `provider_lifecycle_controller_test.rs` / `terminal.rs` / `terminal_test.rs` / `workflow.rs`）、計測本体 `test_クライアントws_往復レイテンシ実測` は `src-tauri/tests/client_api/mod.rs:285` にある。この本体は独立した統合テスト target に属し library の test module には含まれないため、`latency.md:33` の `--lib` 指定は現行の target 構成でこの計測テストを選択しない。
- この周までに解消となった Thread: `9aa0d9fc-556f-43af-980e-ea26bc25754c`、`ecb5e7a5-ee02-41f3-99fd-ab8cdb07ea45`、`6a2ee4a2-1e11-485c-b988-1c148327a251`、`1324cbdd-8e9b-4be6-a53c-2ebd7cf67dbe`、`5500c3f7-2f71-4b94-9b6d-f745011542d3`、`127ad23b-57a1-42f2-a53a-6e0a68847d9c`、`eef79195-8b02-4836-a9b4-7e4ed3a32a8f`。
- 見送り（`[DEFERRED]`）および不成立（`[REJECTED]`）とした Thread はない。
- 今周の Requirements の変更はない。Behavior の変更もない。要件 ID と Behavior ID の対応は R-001〜R-015 と B-001〜B-020 で欠落なく対応しており、修正を要する誤り・不足・矛盾はない。

## 変える部分

- レイテンシ記録から計測テストへの参照と再実行記述の更新: 実測記録の参照先が現存し往復レイテンシの計測本体へ到達すること、および再実行記述が現行の target 構成でその計測テストを選択すること（`#[ignore]` 付きのため ignored の指定が保たれること）。根拠: Thread `6577bf21-2b8d-44d4-b2ee-11d7315528f1` の `[FIX_POLICY]`、R-012「ws の往復レイテンシの実測値が `docs/specs/issues-1199/` 配下の文書に記録され、後続の予算判断から参照できる」、B-016。対象は `latency.md:38` の参照先と `latency.md:33` の再実行記述に限る。ルート: 委任

## 固定するルート

- 今周に新しく固定する実装上の指定はない。
- design-01 で固定したルートを今周も維持する。local API（`adaptor/controller/api/`、axum）上への ws route 新設、`/v1/terminal` を雛形とするエンベロープ、`Sec-WebSocket-Protocol` bearer 認証、`CommandRequest` / `CommandResponse` と typed push、`attachment_id`＋`sequence` / `ack` と frame 上限・分割の規約、`CommandRouter` の transport 非依存化、push sink の単一化と対象 8 イベント、クライアント token の `TerminalStreamEndpoint` 経路、`[server]` 設定の削除対象、旧 ws shell の非再利用。

## 変えないもの

- `latency.md` に記録済みの実測値、計測条件、計測範囲と判断上の制限の記述。理由: Thread `6577bf21-2b8d-44d4-b2ee-11d7315528f1` の `[FIX_POLICY]` が受入条件としてこれらの保持を定めており、今周の変更対象を参照先と再実行記述に限っているため。
- 往復レイテンシの許容閾値と GO/NO-GO 判定。理由: 同 `[FIX_POLICY]` が追加しないことを受入条件に含めており、Requirements の Q-004 と一致するため。
- ws 経由にした 2 本（`get_current_branch` / `workflow-execution-changed`）以外の desktop 機能の経路と結果。理由: R-010 が変更前と同じ経路・同じ結果での動作を要求しており、今周の変更は文書だけで既存経路の結果を変えないため（design-01 から維持）。
- `menu-event` / `native-file-drop` の既存経路。理由: UI shell の事象であり push sink 単一化の対象外と決まっているため（R-006 / B-009、design-01 から維持）。
- エンベロープ定義としての frame 上限・分割の規約。理由: R-004 / B-006 が引き続きこの規約を要求するため（design-03 から維持）。
- A2 で行う stream フロー制御の送受信時の適用。理由: Non-goals および Q-006 と一致し、今周の変更がこの適用を必要としないため（design-02 から維持）。

## 未確定・リスク

この周までに自動判断した箇所は次のとおり。いずれも人間の確認を経ていない。`[DEFERRED]` で人間へ渡した件はない。

- 自動判断（Q-009、design-03 から継続）: frame 上限値は、renderer へ渡す接続情報に含めない。接続情報としての配布が必要と判断された場合、Non-goals と Q-009 が変わる。
- 自動判断（Q-008、design-02 から継続）: 既存の設定ファイルに残る `[server]` セクションは、読み込み時に除去せず、次に設定を保存したときに除去する。内容と影響は design-02 の「未確定・リスク」を参照する。
- 自動判断（Q-001〜Q-007、design-01 から継続）: ws 経由にする req/resp 1 本を `get_current_branch`、push 1 本を `workflow-execution-changed` に確定した件、レイテンシ実測値の記録先を `docs/specs/issues-1199/` 配下に確定し許容値（閾値）を定めていない件、ブランチ取得失敗時に Tauri invoke へフォールバックしない件、stream フロー制御と frame 上限・分割を A1 ではエンベロープ定義の規約までとする件、クライアント token を既存の非 master token と同一の 1 本とする件。内容と影響は design-01 の「未確定・リスク」を参照する。
