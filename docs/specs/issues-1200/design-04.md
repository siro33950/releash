# Design 04

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `28f0eeed`。作業ブランチは `feat/issues/1200`。
- 直前の Design は `docs/specs/issues-1200/design-03.md`。開始状態は、design-01〜design-03 の周で実装した未コミットの変更を含む現在の作業ツリーである。
- Requirements の変更: R-014「クライアント ws で受理された監視開始の要求（`start_watching` / `start_git_dir_watching`）への応答をクライアントが受け取る前にそのクライアント ws 接続が切断された場合、その要求で開始された監視は backend に残らない」、Scope の変更するもの「クライアント ws の切断により応答を受け取れなかった監視開始の要求で開始された監視を、backend に残さないこと」、Assumptions の自動判断（監視開始の要求への応答を受け取れなかった場合）が追加された。Behavior に B-016 と対応表の R-014→B-016 が追加された（Thread 058bbf38-6661-4bd4-b84b-ff3d417f4d5b）。R-001〜R-014 と B-001〜B-016 は対応表で欠落なく対応している。
- この周までに解消した Thread: design-03 の開始状態に挙げた 13 件に加え、44866668-eabd-4091-bb7a-fe66595f7e7e、919f446f-9135-4423-9a56-c5910895a51d、9a055b1a-f929-44a9-b5d3-17b3d6bae984、9d5aa0fb-83ac-40a3-a371-6d4cd7f598c3、b39a9a64-b874-4209-9c18-3b288182bf81、b6843814-db82-4943-9ae9-fc4d30d8ad10、eb55012c-92e4-445d-9d04-ab7f8fd2c5e2。見送りとなった Thread はない。
- open Thread は 9 件で、すべて `[FIX_POLICY]` 付きである。うち 7c9df8e1-3874-4131-906a-fd549981a520、d01766f0-7b64-47ca-9f41-acf05879dbba、4769fbe8-7edd-419b-933f-bf73d245ab8d は design-03 の変える部分に挙げた項目で、`[STILL_OPEN]` により未充足の部分が示されている。
- レビューで基準 `28f0eeed` から存在し今回の差分が導入・悪化していないと判定された 2 件（`src/hooks/useWorkspaceStateCache.ts` が保存完了前に dirty を解除する件、`src/hooks/useAutomation.ts` の workflow 連続選択で古い応答が新しい選択を上書きする件）は Thread になっておらず、今周の変える部分に含めない。

## 変える部分

- 応答前に切断された監視開始要求の監視の回収: クライアント ws で受理された `start_watching` / `start_git_dir_watching` への応答をクライアントが受け取る前にその接続が切断された場合、その要求で開始された監視（非 repository のファイル監視と repository の git dir 監視の双方）が backend に残らないようにする。受理済み command を切断後も完了まで実行する性質（解消済み Thread 88f0cfb5-2a03-4a3e-913d-64b9ff11850b の方針）と、応答を受け取った監視を切断によって停止しないことは維持する。根拠: R-014「…その要求で開始された監視は backend に残らない」、B-016、Thread 058bbf38-6661-4bd4-b84b-ff3d417f4d5b。ルート: 委任
- 同じ worktree への連続した workspace state 保存の受信順適用: 同じ worktree への `save_workspace_state` をクライアント ws で続けて送った場合、すべての保存の完了後に backend が保持し永続化される状態が最後に送った要求の状態になり、再起動後にその状態が復元されるようにする。根拠: R-010「desktop の UI 機能が対象ドメインの command を呼ぶ操作は、クライアント ws 経由の要求で行われ、変更前と同じ結果になる」、B-011、Thread 95748435-597e-4c0e-9d19-0c3bf1644cea。ルート: 委任
- push 購読の未読溢れを接続全体の失敗へ波及させない: push の未読が購読容量を超えても、同じ接続上の未完了の req/resp が接続切断によるエラーにならず、attach 中の terminal が detach されないようにする。`workflow-execution-changed` を購読する画面が backend の現在状態と後続の変化を反映することは変更前と同じにする。根拠: R-010、B-011、R-004「…req/resp と push と同じ 1 本のクライアント接続上で行える」、B-004「同じ接続で req/resp と push のやり取りが続けられる」、Thread 308cacd0-135f-4199-81af-211b1df61a22。ルート: 委任
- Exit なしの attachment stream 終了の desktop への伝達: 表示中の terminal の attachment stream が Exit を配送せずに backend 側で終了した場合（再同期時の surface 取得失敗、subscription の終了等）、desktop がその終了を観測し、変更前と同じく再 attach・再同期による回復、または terminal のエラー表示へ進むようにする。通常の Exit 配送と backend attachment の解放（解消済み Thread 6dda6197-0427-4bc2-9450-6ffe125e0bc8）は変えず、同じ接続上の他の req/resp・push・attachment は継続させる。根拠: R-012「desktop の terminal は、クライアント ws が予期せず切断された後…接続が確立すると再 attach し、出力を再同期して表示と入力を再開する」、B-013、R-010、B-011、Thread 86b3b120-74d5-42bf-8010-e25a92fc697a。ルート: 委任
- application_lifecycle のエラー内容の保持: application_lifecycle の command がエラーを返した場合、クライアント ws と Tauri invoke のどちらから呼んでも、`type`・`message`・`correlation_id`・`failure` が変更前の既存 DTO（`adaptor/protocol/application_operation_v1.rs` の Serialize）の serde 表現と同じ意味で届き、終了監督 UI 等に変更前と同じエラー説明が表示されるようにする。変更前から message を持たない `ApplicationUnavailable` は対象外。根拠: R-002「クライアント ws から呼んだ command は…Tauri invoke から呼んだ場合と同じ usecase の結果（成功値またはエラー）を返す」、B-002、R-010、B-011、Thread 0dd55b97-100b-4d7c-bcaf-c7d17c6fd7ba。ルート: 委任
- workspace tree の過去試行の Node 表現の保持: `list_workspace_worktree_nodes` と `get_workspace_tree_selection_reconciliation` の結果の `pastAttempts` の各要素が、変更前と同じ `kind: "node"` を含む Node の表現で desktop に届き、過去試行が Node 行として表示される（children のない過去試行でも描画が失敗しない）ようにする。根拠: R-002、B-002、R-010、B-011、Thread 163eb233-23b7-4acb-b568-1e8793fd036c。ルート: 委任
- 到達しない外部タグ形式の出力分岐の削除（未充足分）: `.proto` の `variant` oneof を持つ message はすべて `json_tag` または `json_untagged` を持つため到達しない、どちらも持たない oneof を外部タグ形式で出力する分岐（Rust codec `controller/api/protocol/json.rs`、TypeScript codec `src/lib/clientJson.ts`、型生成スクリプト `scripts/generate-client-protocol.mjs`）を残さず、被覆 command・push・stream の送受信結果は変えない。根拠: R-003 の実装、design-03 の変える部分「未使用の変換形式の削除」、Thread 7c9df8e1-3874-4131-906a-fd549981a520。ルート: 委任
- workflow 実行ログの固定メタデータの生成型への結び付け（未充足分）: `get_workflow_execution_log` の結果について、`.proto` に固定項目として定義された `event` / `execution_id` / `timestampMs` を、JSON の値（`WorkflowEventView`）から文字列キーの取り出しと実行時の型検査で取り直す状態を解消し、固定メタデータを生成型へ型付きで結び付ける。`get_workflow_execution_log` の結果と R-003（バイナリ frame）・R-002 の parity は変えない。根拠: 「固定するルート」の判断①（design-01 から維持）「`.proto` を protocol の正とし、Rust / client の型はそこから生成する」、design-03 の変える部分「Rust handler の入出力契約の生成型への結び付け」、Thread d01766f0-7b64-47ca-9f41-acf05879dbba。ルート: 「固定するルート」の判断①
- `get_application_startup_outcome` の parity 検証の入口（未充足分）: `get_application_startup_outcome` の R-002 parity テストが、本番の Tauri 入口（`application_lifecycle::invoke_handler` の shell 入口）を通して結果の一致を確認するようにする。起動失敗時に必要な `get_application_startup_outcome` / `quit_after_startup_failure` の入口の削除は含めない。根拠: R-002、B-002、design-03 の変える部分「共有 dispatch 登録済み command の旧 Tauri 入口の削除と parity 検証の入口」、Thread 4769fbe8-7edd-419b-933f-bf73d245ab8d。ルート: 「固定するルート」の検証方法（design-01 から維持）

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
- 監視開始の要求への応答を受け取れなかった場合: 応答をクライアントが受け取る前に接続が切断された監視開始の要求で開始された監視を backend に残さない（R-014）。切断時に接続上の全監視を停止すること、接続の確立後に desktop が監視を開始し直すことは要求しない。
- `get_terminal_stream_endpoint` を `/v1/terminal` route とともに削除する。
- terminal の Tauri Channel fallback の削除をこの変更で行う。Issue #1201 も同じ撤去を A-flip の作業に挙げている。
- 作業単位を Issue #1200 の対象ドメインすべてとする。
