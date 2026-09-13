# Design 03

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `28f0eeed`。作業ブランチは `feat/issues/1200`。
- 直前の Design は `docs/specs/issues-1200/design-02.md`。開始状態は、design-01・design-02 の周で実装した未コミットの変更を含む現在の作業ツリーである。
- Requirements の変更: R-013「desktop の terminal は、クライアント ws 上の attach 要求にエラー応答が返った場合、変更前と同じくその応答のエラーを terminal のエラーとして表示する」と B-015、対応表の R-013→B-015 が追加され、Assumptions に attach 要求へのエラー応答の自動判断が加わった（Thread e4cdbbac-1b68-438b-a578-1d4debba18b1）。開始状態の `src/hooks/useTerminal.ts` は attach のエラーを terminal のエラーとして表示しており R-013 / B-015 を満たすため、変える部分に含めない。R-001〜R-013 と B-001〜B-015 は対応表で欠落なく対応している。
- この周までに解消した Thread: e4cdbbac-1b68-438b-a578-1d4debba18b1、56980c07-2a81-4dd7-9ed8-dfcbd7fb1814、61e741a5-5396-43c2-b4e2-b08f762d2fe0、657a8904-7d8f-45bf-b25b-da320850505b、6dda6197-0427-4bc2-9450-6ffe125e0bc8、730d0086-c2bb-48b6-b58d-8c7ac94a87de、7c015674-637f-4ad6-80da-cbaa63822840、83ec7b9c-b29f-4db7-b664-236aee1bafac、88f0cfb5-2a03-4a3e-913d-64b9ff11850b、bdfc10be-3f28-4643-99e1-ecac13d910b1、d065aed3-741d-4ae2-99de-a9006f9dddd9、f276349d-a7a9-4079-ae20-c5b8244fa45c、fcbd3ef1-94e5-4fd1-910a-d3ef2ef230f0。見送りとなった Thread はない。
- open Thread は 10 件で、すべて `[FIX_POLICY]` 付きである。うち 44866668-eabd-4091-bb7a-fe66595f7e7e、d01766f0-7b64-47ca-9f41-acf05879dbba、eb55012c-92e4-445d-9d04-ab7f8fd2c5e2 は design-02 の変える部分に挙げた項目で、`[STILL_OPEN]` により未充足の部分が示されている。
- レビューで基準 `28f0eeed` から存在し今回の差分が導入・悪化していないと判定された 2 件（repository の読み取りで controller が Entity から DTO を生成している件、`src/hooks/useWorktreeList.ts` が PR 情報でマージ状態を上書きしている件）は Thread になっておらず、今周の変える部分に含めない。

## 変える部分

- workspace state 保存要求の ws での受理: 実際の UI が生成する workspace state（`layout.reviewCollapsed` / `diffOnlyMode` を含む）の `save_workspace_state` 要求がクライアント ws の送信前に拒否されず受理され、変更前に保存されていた項目が backend に保存され再起動後に復元されるようにする。両フィールドの永続化は新たに要求しない。根拠: R-010「desktop の UI 機能が対象ドメインの command を呼ぶ操作は、クライアント ws 経由の要求で行われ、変更前と同じ結果になる」、B-011、Thread 9d5aa0fb-83ac-40a3-a371-6d4cd7f598c3。ルート: 委任
- 安全整数範囲外の WorkflowValue の受信で接続を失敗させない: backend が正常に返す安全整数範囲外の整数を含む WorkflowValue を持つ応答・push を desktop が受信しても、クライアント ws 接続が失敗せず、同じ接続上の他の未完了要求と terminal stream が継続し、該当 workflow の状態取得・表示が変更前と同じ結果になるようにする。根拠: R-010、B-011、R-004「req/resp と push と同じ 1 本のクライアント接続上で行える」、B-004、Thread b39a9a64-b874-4209-9c18-3b288182bf81。ルート: 委任
- 新設 protocol 型名からの使用禁止語の除去: 今回新設した `.proto` の message 名（`WorkflowEventLogEntry`、`ListWorkflowEventLogEntry`、`NullableListWorkflowEventLogEntry`）と、そこから生成される Rust / TypeScript 型に `WorkflowEvent` の語彙が含まれないよう `docs/glossary/DOMAIN.md` の正規語に従う名前にし、`get_workflow_execution_log` の結果は変えない。既存 Rust の `WorkflowEventView` 等の改名は含めない。根拠: R-003 の実装、`docs/glossary/DOMAIN.md` 使用禁止語表「WorkflowEvent → durable workflow fact」、Thread 919f446f-9135-4423-9a56-c5910895a51d。ルート: 委任
- 共有 dispatch 登録済み command の旧 Tauri 入口の削除と parity 検証の入口: 共有 dispatch に登録済みの command について、通常起動時に到達しない旧 Tauri 入口関数と `generate_handler` 登録を残さず Tauri 入口を一つに定め、R-002 の parity テストが本番で到達する Tauri 入口を通して結果の一致を確認するようにする。起動失敗時に必要な `get_application_startup_outcome` / `quit_after_startup_failure`、menu、`get_client_endpoint`、`attach_terminal_surface` 等の到達する入口は対象外。根拠: R-002「クライアント ws から呼んだ command は、同じ command を同じ引数で Tauri invoke から呼んだ場合と同じ usecase の結果（成功値またはエラー）を返す」、B-002、`docs/architecture/CONTROLLER.md` の入口分離、Thread 4769fbe8-7edd-419b-933f-bf73d245ab8d。ルート: 「固定するルート」の検証方法（design-01 から維持）
- 未使用の変換形式の削除: `.proto` のどの message・field にも設定されていない `json_tuple` / `json_external` の option 宣言と、Rust codec（`controller/api/protocol/json.rs`）・TypeScript codec（`src/lib/clientJson.ts`）・型生成スクリプト（`scripts/generate-client-protocol.mjs`）の対応分岐を残さず、被覆 command・push・stream の送受信結果は変えない。根拠: R-003 の実装、Thread 7c9df8e1-3874-4131-906a-fd549981a520。ルート: 委任
- 接続破棄による backend attachment 解放のテスト: クライアント接続の破棄により backend attachment が非活性化され、stream と保留 output credit が解放されることを確認するテストを追加し、接続破棄の処理から backend detach を外すとそのテストが失敗するようにする。根拠: R-012「クライアント ws が予期せず切断された後…接続が確立すると再 attach し、出力を再同期して表示と入力を再開する」、B-013、Thread 9a055b1a-f929-44a9-b5d3-17b3d6bae984。ルート: 委任
- FileWatcherGateway の境界テストの規約配置: FileWatcherGateway の変換テストを gateway 実装と同じディレクトリの `gateway/repository/file_watcher_test.rs` に置き、変換と「start が返す ID ＝ FileChange 通知の `watcher_id`」の一致を検証し、ID 配線をずらすとテストが失敗するようにする。controller 配下に gateway の変換テストを残さない。根拠: R-001、R-010 の実装、`docs/architecture/TEST.md`「adaptor/gateway/ 必須: 外部システムとの境界、モデル変換の検証」と `<impl>_test.rs` の配置規約、Thread b6843814-db82-4943-9ae9-fc4d30d8ad10。ルート: 委任
- terminal stream の接続処理の Tauri 入口配下からの除去（未充足分）: `controller/command/client.rs` の `dispatch_stream` が local API 専用の `TerminalConnection` を受けて attach / detach と `attachmentId` の解釈を所有している状態を解消し、ws 接続単位の attachment 表・frame 分割・ack window・stream の attach / detach 処理等の local API 専用処理が Tauri 入口（`controller/command/`）配下に置かれず、local API 入口がそれを `controller/command/` 配下から import しないようにする。根拠: R-004 の実装、`docs/architecture/CONTROLLER.md`「原則」の入口分離、design-02 の変える部分「ws 接続単位の terminal 処理の配置」、Thread 44866668-eabd-4091-bb7a-fe66595f7e7e。ルート: 委任
- Rust handler の入出力契約の生成型への結び付け（未充足分）: 生成 prost message を `into_value` で JSON に戻して文字列キーと手書きの Rust Args（`command/client.rs`、各ドメインの `shared.rs`、`api/client_stream.rs`）で引数を取り直す状態と、結果・push を serde JSON から実行時変換する状態を解消し、被覆 command の必須引数・結果と push payload の型を `.proto` から生成した Rust・TypeScript 型で扱い、command 契約の正を `.proto` にする。R-003（バイナリ frame）と R-002 の parity は維持する。根拠: 「固定するルート」の判断①（design-01 から維持）「`.proto` を protocol の正とし、Rust / client の型はそこから生成する」「push も proto message とする」、design-02 の変える部分「command 引数・結果と push payload の `.proto` 定義」、Thread d01766f0-7b64-47ca-9f41-acf05879dbba。ルート: 「固定するルート」の判断①
- 共有 dispatch の Tauri 入口依存の除去（未充足分）: `register_app` が `AppHandle` を各ドメインへ渡し、`AppHandle` から得た `AppState` で `controller/command/` 配下の `*_shared` を呼ぶ状態と、`AppHandle` を捕捉した emit や data_dir 解決を共有経路に保持する状態（例: `comment/shared.rs`）を解消し、ws 共有 dispatch の各 handler が Tauri command 関数・`AppHandle` の managed state に依存せず usecase 操作を呼び、Tauri 入口と local API 入口が同じ usecase を呼ぶ薄い入口として分離されるようにする。R-002 の parity は維持する。根拠: 「固定するルート」の判断①（design-01 から維持）「req/resp は…エンベロープ…を経由して usecase 共有 dispatch へ渡す」、`docs/architecture/CONTROLLER.md`「どちらの入口も同じ Usecase を呼ぶ」、design-02 の変える部分「共有 dispatch の usecase 直接呼び出し」、Thread eb55012c-92e4-445d-9d04-ab7f8fd2c5e2。ルート: 「固定するルート」の判断①

## 固定するルート

- 今周に新たに固定する実装上の指定なし。
- design-01 の「固定するルート」をすべて維持する（判断①、判断⑧、判断⑦、土台、対象ドメインの単位、terminal_surface、検証方法）。

## 変えないもの

- design-01 の「変えないもの」を維持する（CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口と結果）。

## 未確定・リスク

`[DEFERRED]` で人間へ渡した件はない。「自動判断: 未決」のまま残した要求はない。この周までの自動判断（`requirements.md` の Assumptions）はいずれも人間の確認を経ていない。

### 自動判断

- Q-001: クライアント ws のエンベロープ（req/resp、push、stream）をこの変更で Protocol Buffers バイナリ frame に置き換える。A1（PR #1768）と `docs/specs/issues-1199/` は JSON text frame を前提としている。
- Q-002: desktop の該当 UI 機能の呼び出しをこの変更でクライアント ws 経由へ差し替える。Issue #1201（A-flip）は呼び出し側の差し替えを A-flip の作業に挙げている。
- Q-002 に伴う判断: `get_application_startup_outcome` と `quit_after_startup_failure` の desktop からの呼び出しは Tauri invoke のまま維持する。
- Q-003: telemetry / watcher / application_lifecycle は登録済みの全 command を被覆し、UI shell に残す分の線引きは A-flip で決める。
- Q-004: desktop から呼び出す箇所がない登録済み command 34 本を、他の command と同様にクライアント ws へ被覆する。
- Q-005: Tauri Channel fallback の削除後、クライアント ws の切断・接続失敗の後に接続が確立した時点で terminal を再 attach・再同期する。
- attach 要求へのエラー応答: クライアント ws 上の attach 要求にエラー応答が返った場合、desktop の terminal はその応答のエラーを terminal のエラーとして表示し、接続が続いている間の自動再試行は要求しない（R-013）。
- `get_terminal_stream_endpoint` を `/v1/terminal` route とともに削除する。
- terminal の Tauri Channel fallback の削除をこの変更で行う。Issue #1201 も同じ撤去を A-flip の作業に挙げている。
- 作業単位を Issue #1200 の対象ドメインすべてとする。
