# Context

- 要求の正本: Issue #1201「A-flip: デスクトップ既定トランスポートを ws に切替」。
- 背景資料: milestone 77「02. ローカル server-client 化（基盤）」、Issue #1199（A1）、Issue #1200（A2）、Issue #1203（A-launchd）、`docs/specs/issues-1199/requirements.md`、`docs/specs/issues-1199/latency.md`、`docs/specs/issues-1200/requirements.md`、`docs/specs/issues-1200/design-05.md`、`src/lib/clientSocket.ts`、`src-tauri/src/adaptor/controller/command/terminal_surface/mod.rs`、`src-tauri/src/adaptor/controller/command/code/review_blob.rs`。
- milestone 77 で確定済みの設計判断のうち、本変更が従うもの。
  - 判断①: req/resp は Protocol Buffers で定義したエンベロープ（`request_id`＋`oneof command`）を経由して usecase 共有 dispatch へ渡す。push も proto message とする。`.proto` が protocol の正である。
  - 判断③: クライアント通信は WebSocket 単一トランスポートとする。
  - 判断⑦: クライアント token は discovery file の master token と分けて発行し、master token を renderer へ露出させない。
  - 判断⑧: terminal stream は汎用エンベロープに統合し、クライアント接続は 1 本にする。
  - 判断⑥: strangler 移行（A0 掃除 → A1 walking skeleton → ドメイン被覆 → 既定切替 → デーモン抽出 → 常駐）。本変更は既定切替（A-flip）にあたり、デーモン抽出（#1202）と常駐（#1203）より前に行う。
- milestone 77 の成果は、ローカルで desktop が daemon＋ws で完全動作すること（desktop 1 クライアント。CLI / hook の HTTP 入口は対象外）である。
- Issue #1200 は、`review-blob:` scheme の代替と、telemetry / watcher / application_lifecycle のうち UI shell に残す分の線引きを本変更で扱うと定めている。
- Issue #1203 は、daemon の起動監督（起動・監視・再 spawn・停止）を UI shell の Rust が所有し、WS が確立しない場合も UI shell が失敗理由を取得・表示すると定めている。未確定の変更要求は本変更の結果不明・再送規則に従うと定めている。
- 次の所有は Issue #1201 が定める制約である。
  - 操作の受理・重複判定・結果照会は Rust が所有し、frontend は状態表示と利用者の操作受付を行う。
  - 期限・失敗分類・復旧方針、および各 command の変更要求が冪等か（再実行しても結果が変わらないか）の分類は Rust が所有する。確認間隔、応答期限、操作別の待機期限は設計で具体化する。
  - 再実行で副作用が重複する command の結果確認・重複防止には、既存の操作識別・照会機構を優先して利用する。
- 結果の受け取りが確認されていない変更要求の記録を保持する期間（保持期間）の値は Rust が所有する。
- 要求の送信・待機・再送・期限の扱いは、一般的なサーバクライアント方式と同じ仕組みにし、backend が UI shell から独立しても同じ仕組みで動作するようにする。参照する一般的な仕組みは次のとおり。
  - クライアントから送信されていない要求は、期限内であれば送り直しても重複しない（gRPC transparent retry）。
  - 接続がない間の要求は、期限まで接続の回復を待ち、期限を過ぎれば失敗とする（gRPC wait-for-ready と deadline）。
  - サーバは期限を過ぎた要求の処理を開始しない（gRPC deadline）。
  - サーバに届いた可能性がある非冪等な要求は、冪等であるか、元の要求が適用されていないことを判別できる場合を除き、自動で再送しない（RFC 9110 §9.2.2）。
  - 冪等キーに対応付けた結果の記録には有効期限があり、有効期限を過ぎた記録は破棄される（IETF Idempotency-Key 草案の有効期限、Stripe の冪等キーの 24 時間での破棄）。
- クライアント ws の接続情報（URL と非 master token）はクライアント ws の接続前に取得する必要があり、クライアント ws 上では取得できない。
- 起動 authority が失敗した場合、local API は bind されず、クライアント ws は存在しない。

# Outcome

- 対象者は、desktop 利用者と、デーモン抽出（A-daemon）・常駐（A-launchd）を実装する開発者である。
- 現在、desktop の req/resp の大半はクライアント ws 経由になっているが、backend 状態の push の大半は Tauri event で受け取っており、backend は全 command を Tauri invoke でも受け付け、terminal の Tauri Channel 経路と `review-blob:` URI scheme も残っている。このため desktop は Tauri の IPC なしに backend と通信できず、backend を Tauri 非依存のデーモンへ切り出す前提が揃っていない。またクライアント ws の切断時には応答待ちの要求をすべて失敗として扱い、切断・無応答の後に変更処理が実行されたかを利用者が判断できない。接続を閉じないまま通信が止まった状態を検知する手段もなく、画面が待機中のまま残りうる。
- 変更後は、desktop が backend と行う通信はクライアント ws だけで行われ、Tauri の IPC は UI shell に残す物、クライアント ws の接続情報の取得、起動結果の取得と起動失敗時の処理に限られる。切断・無応答の後も、変更要求が未送信か、結果不明か、結果が確定したかが区別され、結果不明の変更要求の副作用は重複しない。通信路の無応答は有限の期限で検知されて表示され、再接続後は現在の状態が画面へ反映される。

# Current Behavior

最初の周の開始時点（`feat/issues/1201`、`dad6171d`）で、コードを読んで確認した挙動。アプリケーションの起動・テストの実行による確認は行っていない。

## Issue 本文の現状記述との差

- Issue #1201 は「`invoke` を 44 ファイルが直接 import」と記述しているが、これは #1200 着手前の状態である。#1200 の実装（PR #1799〜#1818、`dad6171d` に統合済み）後、`@tauri-apps/api/core` を import する本番ファイルは `src/App.tsx`、`src/components/workspace/WorkspaceList.tsx`、`src/lib/clientSocket.ts` の 3 件である（`src/test/setup.ts` を除く）。
- ws クライアントは `src/lib/clientSocket.ts` の 1 箇所にあり、`src/lib/clientSocket.ts` 以外の本番ファイル 45 件が `invokeClient` を使う。

## desktop に残る Tauri invoke

- `src/lib/clientSocket.ts`: `get_client_endpoint`（クライアント ws の URL と認証 subprotocol の取得）。
- `src/App.tsx`: `get_application_startup_outcome`、`quit_after_startup_failure`、`set_menu_items_enabled`。
- `src/components/workspace/WorkspaceList.tsx`: `close_workspace_node`。この command は backend に登録されておらず（MS87 #1621 で削除）、呼び出しは node の `capabilities.canClose` が真のときだけ表示される操作から行われる。backend は `can_close` を常に `false` で返すため、この操作は表示されない。

## desktop に残る Tauri event

- backend 状態の push のうち、クライアント ws で購読するのは `workflow-execution-changed` だけである。次は Tauri `listen` で購読している。
  - `agent-session-changed`（`src/lib/agentSessionEvents.ts`）
  - `branch-list-sync`（`src/hooks/useWorktreeList.ts`）
  - `file-change`（`src/hooks/useAutomation.ts`、`src/hooks/useGitEventRefresh.ts`）
  - `git-status-changed`（`src/hooks/useGitEventRefresh.ts`）
  - `repo-paths-changed`（`src/hooks/useRepoList.ts`）
  - `review-comments-changed`（`src/hooks/useDiffComments.ts`）
- `repository-snapshot-changed` を購読する本番コードはない。
- `src/components/workspace/WorkspaceList.tsx` は Tauri `emit("branch-list-sync")` で frontend 内の `useWorktreeList` へ通知している。
- UI shell の事象 `menu-event`（`src/hooks/useMenuEvents.ts`）と `native-file-drop`（`src/hooks/useNativeFileDrop.ts`、`src/components/panels/TerminalPanel.tsx`）は Tauri `listen` で購読している。
- backend の push sink（`adaptor/gateway/push.rs`）は、8 つの push を Tauri emit とクライアント ws の双方へ配っている。

## UI shell の plugin

- dialog（`src/App.tsx`）、opener（`WorkspaceList.tsx`、`src/lib/terminalLinkActivation.ts`）、updater＋process relaunch（`src/hooks/useUpdateChecker.ts`）、autostart（`src/hooks/useAppSettings.ts`）を Tauri plugin で使う。

## backend に残る Tauri 経路

- `adaptor/controller/command/mod.rs` の `CommandRouter` は、全ドメインの `COMMAND_NAMES`（menu と client を含む）を Tauri invoke handler に登録している。menu（`set_menu_items_enabled`）と client（`get_client_endpoint`）以外は、Tauri invoke から共有 dispatch へ転送される。
- `adaptor/controller/command/terminal_surface/mod.rs` は `attach_terminal_surface` を `tauri::ipc::Channel<TerminalSurfaceStreamItemV1>` を受け取る Tauri command として登録し、terminal の出力を Tauri Channel で配信できる。desktop からこの Tauri command を呼ぶ箇所はない（desktop の terminal はクライアント ws で attach する）。
- `review-blob:` は `adaptor/controller/command/code/review_blob.rs` が登録する Tauri の URI scheme protocol である。review の画像差分表示（`src/hooks/useReviewFileView.ts`、`src/components/panels/ImageDiffViewer.tsx`）は、backend が返す `review-blob://localhost/blob?...` の URL を画像の参照先として使う。`src-tauri/tauri.conf.json` の CSP は `img-src` に `review-blob:` を含む。

## クライアント ws の切断・応答待ち（`src/lib/clientSocket.ts`）

- 接続は `get_client_endpoint` の結果で開き、10 秒以内に開かなければ失敗とする。
- 接続が閉じる、エラーになる、または受信 frame の復号に失敗すると、応答待ちの要求をすべて `Error("Client WebSocket connection closed")` で reject する。要求が送信済みかどうか、backend で受理・完了したかどうかは区別せず、`invokeClient` の reject として他の失敗と同じ形で返す。
- 接続が確立していない状態で要求すると、接続を試み、接続に失敗すると要求を reject する。
- 応答待ちの期限は `get_current_branch` にだけ 10 秒で設けられている。その他の要求は応答が届くまで、または接続が閉じるまで待ち続ける。
- 再接続は、push 購読または接続状態の購読がある場合に、1 秒後に 1 回ずつ試みる。接続の確立時に、push 購読側へ再取得を促し（`onReconnect`）、terminal は再 attach・再同期する。
- 生存確認の仕組みはない。backend は WebSocket の Ping に Pong を返すが、自ら Ping を送らない。desktop は Ping を送らない。接続を閉じないまま通信が止まった場合、接続は開いたままと扱われ、応答待ちの要求は待ち続ける。
- OS のスリープ復帰を扱う処理はない。
- 監視開始（`start_watching` / `start_git_dir_watching`）の応答を受け取る前に切断された場合、backend はその監視を停止する（#1200 R-014）。desktop は接続の確立後に監視を開始し直さない。
- workspace state 保存（`src/hooks/useWorkspaceStateCache.ts`）は、保存要求が失敗（切断による reject を含む）すると未保存のまま保持し、後続の flush・unmount で最新の状態を再び保存する（#1200 design-06）。

## 既存の操作識別・照会機構

- agent session の作成・archive・削除・restore・履歴からの再開（`WorkspaceList.tsx`、`AgentSessionPanel.tsx`）は、desktop が生成した `callerRequestId` を要求に含める。backend はこれを local event store の commit admission と `usecase/agent_session/` で扱う。
- application quit は `callerRequestId` と `operation_id` を持ち、`list_pending_application_attempts` / `acknowledge_application_attempt` / `get_application_quit_operation` で未確認の操作と結果を照会できる。結果を確定できない場合は `OutcomeUnknown` を返す（`usecase/application_lifecycle/operation/caller_journal.rs`、`src/hooks/useApplicationShutdownSupervision.ts`）。
- その他の変更 command に、操作の識別子による重複防止や結果照会の仕組みはない。

## backend の構成

- backend は Tauri アプリと同じプロセスで動作し、独立した daemon は存在しない（#1202 / #1203 は未完了）。

## E2E

- Playwright の結合テスト（`tests/*.spec.ts`）は、`tests/helpers/tauri-mock.ts` が `page.addInitScript()` で `window.__TAURI_INTERNALS__` と `__TAURI_EVENT_PLUGIN_INTERNALS__` を注入する。クライアント ws は `page.routeWebSocket()` で横取りし、応答は注入した Tauri IPC mock の command→返り値の表（`ipcHandler`）から作る。Tauri event の push は mock の `plugin:event|emit` で発火する。

## レイテンシ

- `docs/specs/issues-1199/latency.md` は、debug build・単一クライアント・同一プロセスで計測した `get_current_branch` の往復の実測値（中央値 0.163875 ms、p95 0.200292 ms、p99 0.234084 ms）を記録している。許容閾値と GO/NO-GO 判定は定めていない（#1199 Q-004）。

# Scope / Non-goals

## 変更するもの

- desktop に残る backend 状態の push 購読の、Tauri event からクライアント ws への切替。
- desktop に残る Tauri invoke・Tauri event のうち、UI shell に残す物・クライアント ws の接続情報の取得・起動結果の取得・起動失敗時の処理以外の撤去。
- backend の Tauri invoke 経路（UI shell に残す物・接続情報の取得・起動結果の取得・起動失敗時の処理以外の command の受付）と、terminal の Tauri Channel 経路の撤去。
- `review-blob:` URI scheme の置き換え。
- Playwright の Tauri IPC mock に代わる E2E 経路。
- 変更要求の未送信・結果不明・結果確定の区別と、結果不明の変更要求の重複実行の防止。再実行で副作用が重複する command の結果確認または重複防止、結果の受け取りが確認されていない変更要求の記録の保持期間と保持数の上限到達時の扱い、期限を過ぎた未受理の変更要求の扱いを含む。
- クライアント ws の生存確認、個別要求の期限、無応答の表示、再接続と回復後の状態の再取得・購読復旧・terminal の再同期、スリープ復帰時の扱い。

## 変更しないもの

- UI shell に残す物（dialog（フォルダ選択）、opener、updater＋process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）。クライアント ws に載せない。
- クライアント ws の接続情報の取得をクライアント ws に載せること。
- headless デーモンの抽出（#1202）と、daemon の起動監督・常駐・更新時の切替（#1203）。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口。
- リモートアクセス経路と、複数クライアント。
- `.proto` を protocol の正とするエンベロープの形式（判断①）。

# Requirements

- R-001: desktop が backend と行う req/resp と、backend 状態の push（`agent-session-changed` / `branch-list-sync` / `file-change` / `git-status-changed` / `repo-paths-changed` / `repository-snapshot-changed` / `review-comments-changed` / `workflow-execution-changed`）の受信は、クライアント ws だけで行われ、変更前と同じ結果が画面へ反映される。
- R-002: desktop が Tauri の IPC（command 呼び出し・event・Channel・URI scheme）を使うのは、UI shell に残す物（dialog（フォルダ選択）、opener、updater＋process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）、クライアント ws の接続情報の取得、起動が成功したか失敗したかを判定するための起動結果の取得、クライアント ws が存在しない起動失敗時の起動失敗画面の表示と終了に限られる。
- R-003: backend は、R-002 の用途以外の command を Tauri invoke で受け付けず、terminal の出力を Tauri Channel で配信しない。
- R-004: review の画像差分表示は、`review-blob:` URI scheme を使わず、クライアント ws から取得したデータで行われ、変更前と同じ画像が表示される。
- R-005: Playwright による desktop の結合テストは、クライアント ws に載せた機能を Tauri IPC の mock を介さずに実行する。
- R-006: クライアント ws の要求の往復レイテンシが、A1 と同じ計測条件（`docs/specs/issues-1199/latency.md` に記録した、単一クライアント・debug build での `get_current_branch` の逐次往復）で、p95 1 ms 以下かつ p99 2 ms 以下である。
- R-007: クライアント ws の切断または応答待ちの終了の後、変更要求は、未送信が確定した要求、結果不明の要求、結果が確定した要求のいずれかとして区別される。未送信が確定した変更要求は、その操作の期限内にクライアント ws の接続が回復した場合は元の操作として送られ、期限内に回復しなかった場合は未実行として表示される。結果不明の要求は、未実行または処理失敗として表示されない。
- R-008: 冪等でない変更要求は、結果不明になった場合、新しい操作として自動で再送されない。
- R-009: 利用者が結果不明の変更要求を再試行した場合、副作用が重複しない。backend がその変更要求の記録を保持している間は、その再試行は元の操作と対応付けられる。再実行で副作用が重複する command は、同じ操作の結果を確認できるか、重複を防いで再試行できる。
- R-010: 受理後・完了前、または完了後・応答前にクライアント ws が切断された変更要求のうち結果を確定できるものは、backend がその変更要求の記録を保持している間に接続が回復した場合、元の操作の結果として確定し、画面へ反映される。
- R-011: 接続先の backend の再起動などで変更要求の結果を確定できない場合は、結果不明が表示され、安全を確認できない再実行は行われない。
- R-012: クライアント ws の接続上で生存確認が行われ、通常の push や応答が発生しない待機状態は無応答と判定されない。接続を閉じないまま通信路が無応答になった場合は、有限の期限内に無応答と判定される。
- R-013: 個別要求の応答待ちには操作に応じた期限があり、接続全体の無応答判定とは別に判定される。個別要求の期限超過だけを根拠に、接続全体が切断されず、backend の処理が中断されず、処理失敗と表示されない。
- R-014: 個別要求の応答が遅延しても、同じ接続上の他の要求と terminal の入出力は、その遅延に巻き込まれずに続けられる。
- R-015: 接続の無応答または個別要求の期限超過を検知した場合、画面は待機中のまま残らず、通信状態を確認できないこと、または操作結果を確認できないことが表示される。
- R-016: 接続の無応答を検知した場合、desktop は再接続を試みる。接続の回復後は、必要な状態の再取得・購読の復旧・terminal の再同期が行われ、現在の状態が画面へ反映される。
- R-017: 接続の無応答だけを根拠に、backend が強制終了・再起動されない。
- R-018: OS のスリープから復帰した場合、生存確認を再実施してから無応答が判定される。復帰後の生存確認に backend が応答した場合、接続は無応答と判定されない。
- R-019: 期限を超えてから応答が届いた要求についても、変更処理の副作用は重複せず、画面には現在の状態が反映される。
- R-020: 起動 authority が失敗しクライアント ws が存在しない場合も、変更前と同じく起動失敗画面が表示され、利用者が終了を選ぶとアプリケーションが終了する。
- R-021: 冪等な変更要求は、結果不明になった場合、その操作の期限内に自動で送り直されることがある。送り直された場合も、backend の状態はその要求を一度だけ実行した場合と同じになる。
- R-022: backend は、desktop が結果の受け取りを確認していない変更要求の記録を、処理の完了から保持期間を過ぎた後に破棄する。保持期間は、最長の操作期限より長い。
- R-023: 結果の受け取りが確認されていない変更要求の記録が保持数の上限に達した場合、backend は処理が完了した変更要求の記録を古い順に破棄して、新しい変更要求を受け付ける。処理が完了していない変更要求の記録は破棄されない。新しい変更要求が保持数の上限を理由に受け付けられないのは、処理が完了していない変更要求の記録だけで上限に達している場合に限られる。
- R-024: 保持期間の経過または保持数の上限により記録が破棄された変更要求は、照会または再試行された場合、結果不明として表示され、処理失敗として表示されず、安全を確認できない再実行は自動でも利用者の再試行でも行われない。利用者の再試行が元の操作として受け付けられるのは、記録の破棄後も backend が操作の識別子で重複を防ぐ仕組み（既存の操作識別・照会機構）に載る変更要求だけである。冪等な変更要求であっても、この仕組みに載らないものは、記録の破棄後に利用者が再試行しても再実行されない。
- R-025: 結果不明の変更要求が backend に受理されないままその操作の期限を過ぎた場合、その要求は利用者の操作なしに backend で実行されず、結果不明の表示のままになる。期限を過ぎた後にその要求について自動で行われるのは、結果の照会だけである。期限を過ぎた後にその要求が実行されるのは利用者が再試行した場合だけであり、元の操作として重複を防いで一度だけ実行される。

# Assumptions / Open Questions

- 自動判断: R-002 の Tauri IPC の用途に「起動が成功したか失敗したかを判定するための起動結果の取得」を含めた。開始状態では、起動に成功した main window も起動結果（`get_application_startup_outcome`）を Tauri invoke で取得してから通常画面を表示しており、R-002 を「起動失敗時」に限ると、この判定を別の経路へ移す変更が生じて対象範囲が広がるため、既存の挙動を維持する解釈とした。
- 自動判断: R-024 で、冪等であることを「記録の破棄後も backend が操作の識別子で重複を防ぐ仕組みに載る」ことに含めなかった。人間の決定は再試行を受け付ける対象を「重複を防げる仕組み（caller attempt 等）に載る要求だけ」としており、冪等性は仕組みではなく command の性質であること、記録の破棄後は同じ対象の後続操作の有無を確認できず再実行が確定済みの後続操作を取り消しうること（R-021）から、開始状態の実装の拒否を維持する解釈とした。
- R-011 と B-013 の「接続先の backend の再起動」は、backend が Tauri アプリと同じプロセスで動作し backend だけが再起動する構成が存在しない現在の構成では、desktop を別の世代の backend（起動ごとに異なる識別を持つ backend）へ再接続させた状態で判定する。backend のプロセスを実際に再起動した状態での確認は、デーモン抽出（#1202）と daemon の起動監督（#1203）で扱う。
