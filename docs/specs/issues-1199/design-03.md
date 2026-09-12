# Design 03

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `8f6a107a`。作業ブランチは `feat/issues/1199`。
- 直前の Design は `docs/specs/issues-1199/design-02.md`。design-02 が挙げた 11 件のうち 10 件は作業ツリー上に実装済みであり（`adaptor/controller/api/auth.rs` の bearer subprotocol 集約、`api/client.rs` の受信サイズ境界、`command/mod.rs` の `CommandRouter::resolve` と共有 dispatch 傍受後の検証、`adaptor/protocol/client_test.rs` の `ClientEndpoint` field 名の断定、`src/lib/clientSocket.ts` の再接続、`gateway/repository/notify.rs` の定数・テスト削除、`infrastructure/push.rs` の配線責務移設、`Cargo.toml` の `rand` 直接依存の削除ほか）、Spec 工程ではコードを変更していない。この未コミットの実装を今周の開始状態として扱う。
- 残る 1 件（実通信を伴うテストの配置）は、テスト本体が `src-tauri/tests/client_api/mod.rs` へ移った一方、`src-tauri/src/lib.rs:5-7` の `#[cfg(test)] #[path = "../tests/client_api/mod.rs"]` によって library の unit-test module として取り込まれたままであり、`src-tauri/Cargo.toml` に対応する `[[test]]` 宣言も `tests/` 直下の入口ファイルもない。design-02 の受入条件を満たしていないため、今周の「変える部分」に残る。
- この周までに解消となった Thread: `f4f68b13-310d-488d-b40d-2bd7bc86b6f4`、`1624eeff-d2fe-44d5-9e19-a359af3e407b`、`f929350e-3de4-4473-ac75-0aacec4c94da`、`b11cc564-dfa4-408b-aeda-145176c6f04d`、`923b2b7f-5952-4006-ac1d-8c8f4460e99b`、`545b9b7f-2d3a-4841-9028-62ad27b26f07`、`dc6d9c46-2256-43cc-ac8e-a4a2599a007e`、`f75ba5e2-b4c9-469b-a501-0cc119875c08`、`f0d0e0a9-e6b6-40aa-8cd5-f58475e18731`、`14064710-7ce1-461a-9820-57cc99025283`。
- 見送り（`[DEFERRED]`）および不成立（`[REJECTED]`）とした Thread はない。
- 今周の Requirements の変更は、Scope / Non-goals「変更しないもの」への「frame 上限値の、renderer へ渡す接続情報としての配布」の追加と、Assumptions の Q-009 の追加である。Behavior の変更はない。この Non-goal は開始状態の実装が満たしていないため、「変える部分」に挙げる。

## 変える部分

- 実通信を伴うテストの統合テスト境界への移設の完了: 実 git 操作・実 local API 通信・複数レイヤーを通すケースが、library の unit test ではなく統合テストとして扱われるようにする。`src/` 隣接 module test に残すのは実通信を伴わない serialization / dispatch の検証だけにする。根拠: Thread `9aa0d9fc-556f-43af-980e-ea26bc25754c`、`docs/architecture/TEST.md` の統合テスト配置。現在の断定内容（認証拒否、`request_id` 相関、8 イベントの Tauri / ws 一致、UI shell 事象の非経由、接続上限、push 欠落時の切断）が同じ強さで保たれること。ルート: 委任
- クライアント ws controller の infrastructure 直接依存の解消: ws への push 配信経路を gateway の境界に属させ、controller が具体的な送信基盤へ直接依存しないようにする。根拠: Thread `1324cbdd-8e9b-4be6-a53c-2ebd7cf67dbe`、`docs/architecture/README.md`（infrastructure への依存は adaptor/gateway に限る）、`docs/architecture/GATEWAY.md`（外向き通知は gateway が扱う）。`api/client.rs` が `infrastructure::push::PushSink` を import して依存型に持ち、`subscribe` を直接呼んでいる。B-005 の push 受信、B-008 の 8 イベントの双方到達、B-009 の `menu-event` / `native-file-drop` 非経由が変わらないこと。ルート: 委任
- local API 専用の request/response 型の配置: クライアント ws の要求・応答型を、local API だけで使う型の配置へ移す。根拠: Thread `5500c3f7-2f71-4b94-9b6d-f745011542d3`、`docs/architecture/CONTROLLER.md`（local API だけで使うリクエスト／レスポンス型は `controller/api/protocol.rs` に置き、`adaptor/protocol/` へ上げない）。`ClientRequest` / `CommandRequest` / `CommandResponse` / `CommandOutcome` / `StreamEnvelope` の本番利用は `api/client.rs` の ws 入出力だけである。複数入口で共有する `ClientEndpoint` と `CLIENT_WS_PATH` は移設の対象外。B-003 / B-004 の req/resp 相関、B-006 のエンベロープ規約、B-011 の接続情報が変わらないこと。ルート: 委任
- 接続情報からの frame 上限値の公開の取り消し: renderer へ渡す接続情報から frame 上限値の公開項目を取り除き、その項目を断定するテストも取り除く。根拠: Thread `127ad23b-57a1-42f2-a53a-6e0a68847d9c`（分類 scope）、Requirements の Scope / Non-goals「frame 上限値の、renderer へ渡す接続情報としての配布」と Q-009。`get_client_endpoint` の応答に frame 上限値の項目が含まれないこと。R-004 / B-006 が求める frame 上限と上限超過時の分割の規約はエンベロープ定義として残し、取り消しの対象外とすること。B-011 / B-001 / B-012 / B-013 が引き続き満たされること。ルート: 取り消す範囲は上記に限る。取り消しの実装方法は委任
- command 処理待ちによる push 停止の解消: 同一接続で command の完了を待っている間も push の受信・転送が進むようにする。根拠: Thread `6a2ee4a2-1e11-485c-b988-1c148327a251`、R-003 / B-005。`api/client.rs` の `select` は text を選択したアーム本体で dispatch の完了を待ち、その間 push の受信を進めないため、broadcast 容量を超えた場合の取りこぼしで接続が閉じる。B-001〜B-005 の断定が変わらないこと。ルート: 委任
- 初期購読失敗後の購読回復: 画面が有効な間に接続が利用可能になれば購読が確立し、backend の現在状態と後続の push が画面へ反映されるようにする。根拠: Thread `ecb5e7a5-ee02-41f3-99fd-ab8cdb07ea45`（blocking）、R-009 / B-013。`src/lib/clientSocket.ts` は初期接続に失敗した購読を listeners から取り除いて再 throw し、再接続は確立済みの購読がある場合にしか行わない。購読側の hook も同一 worktree / node の間は購読し直さない。破棄済み購読の callback が後の接続で呼ばれないこと、および R-010 のとおり ws 経由にした 2 本以外の経路・結果が変わらないこと。ルート: 委任
- レイテンシ記録から計測テストへの参照の更新: 実測記録の参照先が現存し、往復レイテンシの計測本体へ到達するようにする。根拠: Thread `6577bf21-2b8d-44d4-b2ee-11d7315528f1`、R-012 / B-016。`docs/specs/issues-1199/latency.md:38` の参照先 `../../../src-tauri/src/adaptor/controller/api/client_test.rs` は存在せず、計測本体は `src-tauri/tests/client_api/mod.rs` にある。記録済みの実測値、計測条件、計測範囲と判断上の制限の記述が変わらないこと。Q-004 のとおり許容閾値や GO/NO-GO 判定を追加しないこと。ルート: 委任

## 固定するルート

- 今周に新しく固定する実装上の指定はない。
- design-01 で固定したルートを今周も維持する。local API（`adaptor/controller/api/`、axum）上への ws route 新設、`/v1/terminal` を雛形とするエンベロープ、`Sec-WebSocket-Protocol` bearer 認証、`CommandRequest` / `CommandResponse` と typed push、`attachment_id`＋`sequence` / `ack` と frame 上限・分割の規約、`CommandRouter` の transport 非依存化、push sink の単一化と対象 8 イベント、クライアント token の `TerminalStreamEndpoint` 経路、`[server]` 設定の削除対象、旧 ws shell の非再利用。

## 変えないもの

- ws 経由にした 2 本（`get_current_branch` / `workflow-execution-changed`）以外の desktop 機能の経路と結果。理由: R-010 が変更前と同じ経路・同じ結果での動作を要求しており、今周の 7 件はいずれも既存経路の結果を変えずに解決できるため（design-01 から維持）。
- `menu-event` / `native-file-drop` の既存経路。理由: UI shell の事象であり push sink 単一化の対象外と決まっているため（R-006 / B-009、design-01 から維持）。
- エンベロープ定義としての frame 上限・分割の規約。理由: Thread `127ad23b-57a1-42f2-a53a-6e0a68847d9c` の `[FIX_POLICY]` が取り消しの対象外と定めており、R-004 / B-006 が引き続きこの規約を要求するため。
- A2 で行う stream フロー制御の送受信時の適用。理由: Non-goals および Q-006 と一致し、今周の 7 件のいずれもこの適用を必要としないため（design-02 から維持）。

## 未確定・リスク

この周までに自動判断した箇所は次のとおり。いずれも人間の確認を経ていない。`[DEFERRED]` で人間へ渡した件はない。

- 自動判断（Q-009、今周に追加）: frame 上限値は、renderer へ渡す接続情報に含めない。正本（Issue #1199、milestone 77 判断⑧）は frame 上限をエンベロープの規約として定めることを求めるだけで、接続情報として配布することを求めていない。接続情報としての配布が必要と判断された場合、Non-goals と Q-009 が変わり、取り消しの判断も変わる。
- 自動判断（Q-008、design-02 から継続）: 既存の設定ファイルに残る `[server]` セクションは、読み込み時に除去せず、次に設定を保存したときに除去する。内容と影響は design-02 の「未確定・リスク」を参照する。
- 自動判断（Q-001〜Q-007、design-01 から継続）: ws 経由にする req/resp 1 本を `get_current_branch`、push 1 本を `workflow-execution-changed` に確定した件、レイテンシ実測値の記録先を `docs/specs/issues-1199/` 配下に確定し許容値（閾値）を定めていない件、ブランチ取得失敗時に Tauri invoke へフォールバックしない件、stream フロー制御と frame 上限・分割を A1 ではエンベロープ定義の規約までとする件、クライアント token を既存の非 master token と同一の 1 本とする件。内容と影響は design-01 の「未確定・リスク」を参照する。
