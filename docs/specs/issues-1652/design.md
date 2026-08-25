# Design

## The actual design

### Architecture

#### 表示文言の owner

利用者向けエラー文言は Rust の operation surface 境界が所有する。usecase error から `AppError::Coded` へ変換する `agent_session/provider_tui.rs` と `code/mod.rs`、`TerminalCommandError` および `write_terminal_surface` / `resize_terminal_surface` の文字列 error へ変換する `terminal_surface/commands.rs`、application lifecycle の tagged error を serialize する `application_operation_v1.rs`、Terminal WebSocket error を組み立てる `api/terminal.rs`、Terminal stream item を wire 型へ変換する `protocol/terminal.rs` が、type、code、操作、または内部原因分類に対応する英語の固定文言を選ぶ。

同じ code が複数操作から返る場合、変換関数は操作 discriminator を受け取る。同一 code × 同一操作は必ず同じ文言になり、frontend は discriminator、code、error shape から文言を組み立てない。

`AppError` の serialize 規則は変更しない。`Internal` はプレーン文字列、`Coded` は `{code, message}` のままとし、code の値も変更しない。application lifecycle の tagged error は手書き `Serialize` で `message` を additive に追加し、既存 `type` と variant 固有フィールドを維持する。`input_unavailable` の domain event と usecase stream item は内部原因だけを持ち、protocol 変換後の wire item だけが表示文言を持つ。表示専用文言を domain / usecase error に持ち込まず、`adaptor/controller/api/error.rs` の `ApiErrorBody` と CLI の契約も変更しない。

#### frontend の共通抽出境界

`src/lib/errorMessage.ts` の `getErrorMessage` を、`unknown` から表示・報告用文字列を得る唯一の共通関数とする。

```ts
export function getErrorMessage(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (
		typeof error === "object" &&
		error !== null &&
		"message" in error &&
		typeof error.message === "string"
	) {
		return error.message;
	}
	return String(error);
}
```

この関数は code や type を読まず、文言の追加・置換を行わない。次の24ファイルが直接使用する。静的テストは production source から実際の利用ファイル集合を導出し、この一覧と完全一致させる。

最後の `String(error)` は任意の frontend 値に対する汎用 fallback として維持するが、backend structured error の契約には含めない。backend の coded / tagged error は string `message` を必須とし、Terminal WebSocket の message 欠落は fallback に渡さず malformed frame として処理する。

- `src/components/workspace/WorkspaceList.tsx`
- `src/hooks/useProviderAvailabilitySettings.ts`
- `src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx`
- `src/components/panels/MarkdownDiffViewer.tsx`
- `src/hooks/useReviewFileView.ts`
- `src/lib/telemetry.ts`
- `src/hooks/useTerminal.ts`
- `src/components/panels/NodeContentView/NodeContentView.tsx`
- `src/hooks/useWorkspaceNodeDetail.ts`
- `src/components/panels/automation/FacetEditor.tsx`
- `src/components/panels/automation/NameInputDialog.tsx`
- `src/lib/workflowExecutionActions.ts`
- `src/hooks/useUpdateChecker.ts`
- `src/components/panels/SettingsModal.tsx`
- `src/hooks/useAutomation.ts`
- `src/screens/useWorktreeGitActions.ts`
- `src/hooks/useWorkflowConfig.ts`
- `src/hooks/useAppSettings.ts`
- `src/components/workspace/DeleteWorktreeDialog.tsx`
- `src/components/panels/DiffToolbar.tsx`
- `src/hooks/useWorkspaceTreeNodes.ts`
- `src/contexts/ReviewThreadHandoffContext.tsx`
- `src/hooks/useNotionSettings.ts`
- `src/hooks/useApplicationShutdownSupervision.ts`

`useDiffOperations.ts` の `STALE_REVIEW_GROUP_TARGET` 回復分岐と、`useApplicationShutdownSupervision.ts` の pending attempt cursor 回復分岐は、表示文言ではなく機械可読状態による回復判断なので維持する。shutdown は message 部分一致を使わず、非 null cursor を渡した `list_pending_application_attempts` が `type: "invalid_request"` を返した場合だけ cursor をリセットする。

`terminalStreamSocket.ts` は `TerminalWsErrorV1.message` を直接 `useTerminal.ts` へ渡す。この経路は unknown 値からの抽出を行わないため `getErrorMessage` の24ファイル一覧には含めない。`useTerminal.ts` の `onTerminalError` は `AgentSessionPanel.tsx` と `RightSidebarBottom.tsx` に配線し、両画面の `role="alert"` へ同じ文言を渡す。

#### Terminal の失敗・回復通知

`useTerminal.ts` の `onTerminalError` は `(message: string | null) => void` である。Terminal の失敗時は表示する文言を渡し、初期化・attach が完走した時点と、再同期用 attach が成功した時点では `null` を渡す。exit 済み surface への attach も完走としてクリアし、`onTerminalReady` だけを稼働状態に限定する。再同期成功の通知は component が mount 済みで `attemptEpoch === attachmentEpoch` の場合だけ行い、古い attachment epoch の成功で現行の失敗表示を消さない。

ack IPC 失敗、`input_unavailable`、WS stream error、stream item 適用失敗、Channel fallback の write 失敗は、失敗を観測した component が mount 済みで attachment epoch が現行の場合に `recoverAttachment()` を起動する。引数なしの回復要求は、`recoveringSinceEpoch === attachmentEpoch` の間は再入しない。WS の予期しない切断だけは失敗した epoch を渡し、snapshot 前に recovery socket 自体が切断した場合の既存再入を維持する。

`AgentSessionPanel.tsx` は Terminal 専用の `terminalError` state をライフサイクル操作用の `error` state から分離する。`terminalError` は Terminal 分岐だけで描画し、paused / archived を含む他の分岐へ持ち越さない。`RightSidebarBottom.tsx` も Terminal 専用 state で同じ callback 契約を消費する。

#### frontend 内部例外との分離

- `ReviewThreadHandoffContext.tsx` は `build_review_thread_handoff` の reject と `navigator.clipboard.writeText` の例外を別の catch で扱う。前者は backend 文言のみ、後者は clipboard 操作の文脈を付ける。
- `useTerminal.ts` は Terminal backend command の reject を `TerminalBackendCommandError` で識別し、その message を無加工で表示する。renderer 内部で発生した初期化例外には `Failed to initialize terminal: `、再同期例外には `Failed to resynchronize terminal: ` を付ける。再同期中の `detach_terminal_surface` も backend command 境界でラップする。
- Channel fallback の `write_terminal_surface` reject は `getErrorMessage` で抽出した Rust 所有の message を無加工で表示し、WS stream error と同じ recovery を起動する。`resize_terminal_surface` の reject も同じ Rust 所有の message を持つが、renderer は size 同期の失敗を表示面へ出さず console にだけ記録する。WS stream error は受信済み message を無加工で表示し、stream item 適用失敗は renderer 内部例外として `Failed to apply terminal stream item: ` を付けた後、いずれも recovery を起動する。
- `ack_terminal_surface_output` は業務エラーを返さないため、IPC 失敗には `Failed to acknowledge terminal output: ` を付ける。

### Interface

#### wire 契約

- `AppError::Internal` と `Result<_, String>` の reject 値はプレーン文字列である。
- `AppError::Coded` と `TerminalCommandError` の reject 値は `{code: string, message: string}` である。
- application lifecycle の tagged error は `{type: string, message: string, ...variantFields}` である。
- Terminal WebSocket の error response は `{status: "error", id, error: {code, message}}` である。
- Terminal attachment stream の入力不能通知は `{type: "input_unavailable", session_key, message}` であり、内部原因分類を wire に含めない。
- code は既存値を維持し、機械可読な回復分岐に使用できる。
- type は既存値を維持し、shutdown cursor 回復などの機械可読分岐に使用できる。
- message はそのまま表示できる英語の利用者向け文言である。
- `attach_terminal_surface` は `recovery: bool` を受け取り、初回 attach と再同期の表示操作を Rust 側で区別する。成功時の応答と stream 契約は変更しない。

#### application lifecycle tagged error の type と message

| error type | type | message | 維持する variant field |
| --- | --- | --- | --- |
| `OperationApplicationErrorDtoV1` | `invalid_request` | `Releash could not access application operation history because the request is invalid.` | — |
| `OperationApplicationErrorDtoV1` | `payload_conflict` | `The application operation request conflicts with an earlier request. Refresh and try again.` | — |
| `OperationApplicationErrorDtoV1` | `shutdown_in_progress` | `Releash could not update application operation history while shutdown is in progress. Try again after Releash restarts.` | — |
| `OperationApplicationErrorDtoV1` | `internal` | `Releash could not access application operation history. Try again.` | `correlation_id` |
| `ApplicationQuitErrorDtoV1` | `invalid_request` | `Releash could not start the application quit because the request is invalid.` | — |
| `ApplicationQuitErrorDtoV1` | `payload_conflict` | `The application quit request conflicts with an earlier request. Refresh and try again.` | — |
| `ApplicationQuitErrorDtoV1` | `capacity_exceeded` | `Releash could not start another application quit. Wait for the current operation and try again.` | — |
| `ApplicationQuitErrorDtoV1` | `internal` | `Releash could not complete the application quit. Try again.` | `correlation_id` |
| `ApplicationQuitLookupErrorDtoV1` | `invalid_request` | `Releash could not check the application quit because the request is invalid.` | — |
| `ApplicationQuitLookupErrorDtoV1` | `not_found` | `The application quit operation is no longer available.` | — |
| `ApplicationQuitLookupErrorDtoV1` | `query_busy` | `Releash is still checking the application quit. Try again.` | — |
| `ApplicationQuitLookupErrorDtoV1` | `deadline_exceeded` | `Checking the application quit took too long. Try again.` | — |
| `ApplicationQuitLookupErrorDtoV1` | `storage_unavailable` | `Releash could not access the application quit operation. Try again.` | `failure` |
| `ApplicationQuitLookupErrorDtoV1` | `internal` | `Releash could not check the application quit. Try again.` | `correlation_id` |
| `CurrentShutdownErrorDtoV1` | `internal` | `Releash could not check the current application shutdown. Try again.` | `correlation_id` |
| `ShutdownPlanQueryErrorDtoV1` | `invalid_request` | `Releash could not load the shutdown plan because the request is invalid.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `not_found` | `The shutdown plan is no longer available.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `details_compacted` | `The shutdown plan details are no longer available.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `cursor_mismatch` | `The shutdown plan changed while it was loading. Reload the plan and try again.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `cursor_expired` | `The shutdown plan page expired. Reload the plan and try again.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `query_busy` | `The shutdown plan is busy. Try again.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `deadline_exceeded` | `Loading the shutdown plan took too long. Try again.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `response_too_large` | `The shutdown plan is too large to load.` | — |
| `ShutdownPlanQueryErrorDtoV1` | `storage_unavailable` | `Releash could not access the shutdown plan. Try again.` | `failure` |
| `ShutdownPlanQueryErrorDtoV1` | `internal` | `Releash could not load the shutdown plan. Try again.` | `correlation_id` |
| `ShutdownDetailsMutationErrorDtoV1` | `invalid_request` | `Releash could not compact the shutdown details because the request is invalid.` | — |
| `ShutdownDetailsMutationErrorDtoV1` | `internal` | `Releash could not compact the shutdown details. Try again.` | `correlation_id` |
| `RecoveryActionCommandErrorDtoV1` | `invalid_request` | `Releash could not resolve the shutdown target because the request is invalid.` | — |
| `RecoveryActionCommandErrorDtoV1` | `not_found` | `The shutdown target action is no longer available. Reload the shutdown plan and try again.` | — |
| `RecoveryActionCommandErrorDtoV1` | `storage_unavailable` | `Releash could not save the shutdown target action. Try again.` | `failure` |
| `RecoveryActionCommandErrorDtoV1` | `internal` | `Releash could not resolve the shutdown target action. Try again.` | `correlation_id` |

#### 操作に依存しない AppError の code と message

| code | message |
| --- | --- |
| `PROVIDER_AVAILABILITY_INVALID_EXECUTABLE` | `Enter a Provider executable command name or path.` |
| `PROVIDER_AVAILABILITY_CONFIG_UNAVAILABLE` | `Releash could not access the Provider executable setting. Try again.` |
| `PROVIDER_AVAILABILITY_REFRESH_UNAVAILABLE` | `Releash could not refresh Provider CLI availability. Try again.` |
| `PROVIDER_AVAILABILITY_CORRUPT` | `Releash could not read Provider CLI availability. Restart Releash and try again.` |
| `AGENT_SESSION_PROVIDER_UNAVAILABLE` | `The selected Provider is unavailable. Check its executable and try again.` |
| `AGENT_SESSION_STORAGE_UNAVAILABLE` | `Releash could not access saved AgentSession data. Try again.` |
| `AGENT_SESSION_LAUNCH_UNAVAILABLE` | `Releash could not complete the Provider operation for this AgentSession. Try again.` |
| `AGENT_SESSION_TERMINAL_UNAVAILABLE` | `Releash could not complete the Terminal operation for this AgentSession. Try again.` |
| `AGENT_SESSION_CORRUPT` | `Releash could not continue because the AgentSession data is invalid.` |
| `AGENT_SESSION_NOT_FOUND` | `The AgentSession is no longer available.` |
| `AGENT_SESSION_INVALID_OPERATION` | `This operation is not available for the AgentSession in its current state. Refresh and try again.` |
| `AGENT_SESSION_INVALID_REQUEST` | `Releash could not load the AgentSession because the request is invalid.` |
| `AGENT_SESSION_HISTORY_INVALID_REQUEST` | `Releash could not load AgentSession history because the request is invalid.` |
| `AGENT_SESSION_HISTORY_UNAVAILABLE` | `Releash could not load AgentSession history. Try again.` |
| `AGENT_SESSION_HISTORY_CORRUPT` | `Releash could not load AgentSession history because its saved data is invalid.` |
| `PROVIDER_HOOK_HEALTH_INVALID_REQUEST` | `Releash could not load Provider Hook health because the request is invalid.` |
| `PROVIDER_HOOK_HEALTH_STORAGE_UNAVAILABLE` | `Releash could not load Provider Hook health. Try again.` |
| `PROVIDER_HOOK_HEALTH_CORRUPT` | `Releash could not load Provider Hook health because its saved data is invalid.` |
| `STALE_REVIEW_GROUP_TARGET` | `The review changed before the operation completed. Reload the review and try again.` |

#### 操作に依存する AgentSession code と message

| code | command / operation | message |
| --- | --- | --- |
| `AGENT_SESSION_INVALID_PROVIDER` | Provider executable の update / reset | `Select a valid Provider.` |
| `AGENT_SESSION_INVALID_PROVIDER` | `create_agent_session` | `Select a Provider before starting the AgentSession.` |
| `AGENT_SESSION_INVALID_PROVIDER` | `resume_agent_session_history_candidate` | `Select a Provider before resuming the AgentSession.` |
| `AGENT_SESSION_INVALID_INPUT` | `create_agent_session` | `Releash could not start the AgentSession because the request is invalid.` |
| `AGENT_SESSION_INVALID_INPUT` | `resume_agent_session_history_candidate` | `Releash could not resume the AgentSession because the request is invalid.` |
| `AGENT_SESSION_CONFLICT` | `create_agent_session` | `The AgentSession could not be started because the request conflicts with current state or its Provider session is already in use. Refresh and try again.` |
| `AGENT_SESSION_CONFLICT` | `resume_agent_session_history_candidate` | `The AgentSession could not be resumed because it changed or its Provider session is already in use. Refresh and try again.` |
| `AGENT_SESSION_CONFLICT` | AgentSession lifecycle update | `The AgentSession could not be updated because it changed or its Provider session is already in use. Refresh and try again.` |

#### TerminalCommandError の code、操作、message

| code | command / operation | message |
| --- | --- | --- |
| `CAP_REACHED` | `get_or_spawn_terminal_surface` / 初期化 | `Terminal limit reached. Close an open Terminal and try again.` |
| `PTY_ERROR` | `get_or_spawn_terminal_surface` / 初期化 | `Terminal initialization failed. Try again.` |
| `INVALID_REQUEST` | `get_or_spawn_terminal_surface` / 初期化 | `Terminal initialization failed because the request is invalid.` |
| `CAP_REACHED` | `get_terminal_surface` または `attach_terminal_surface(recovery=false)` / attach | `Terminal limit reached. Close an open Terminal and try again.` |
| `PTY_ERROR` | `get_terminal_surface` または `attach_terminal_surface(recovery=false)` / attach | `Terminal attachment failed. Try again.` |
| `INVALID_REQUEST` | `get_terminal_surface` または `attach_terminal_surface(recovery=false)` / attach | `Terminal attachment failed because the request is invalid.` |
| `CAP_REACHED` | `attach_terminal_surface(recovery=true)` / 再同期 | `Terminal limit reached. Close an open Terminal and try again.` |
| `PTY_ERROR` | `attach_terminal_surface(recovery=true)` / 再同期 | `Terminal resynchronization failed. Try again.` |
| `INVALID_REQUEST` | `attach_terminal_surface(recovery=true)` / 再同期 | `Terminal resynchronization failed because the request is invalid.` |

`write_terminal_surface` は既存の `Result<(), String>` を維持し、gateway failure では `Terminal input could not be sent. Try again.`、owner 変換失敗では `Terminal input could not be sent because the request is invalid.` を返す。`resize_terminal_surface` も既存の `Result<(), String>` を維持し、gateway failure では `Terminal resize failed. Try again.`、owner 変換失敗では `Terminal resize failed because the request is invalid.` を返す。

#### TerminalWsErrorV1 の code、操作、message

| code | WebSocket operation | message | 備考 |
| --- | --- | --- | --- |
| `PTY_ERROR` | attach | `Terminal attachment failed. Try again.` | Tauri の attach と同一 |
| `INVALID_REQUEST` | attach | `Terminal attachment failed because the request is invalid.` | Tauri の attach と同一 |
| `PTY_ERROR` | write | `Terminal input could not be sent. Try again.` | Tauri の write と同一 |
| `INVALID_REQUEST` | write | `Terminal input could not be sent because the request is invalid.` | Tauri の write と同一 |
| `PTY_ERROR` | resize | `Terminal resize failed. Try again.` | Tauri の resize と同一 |
| `INVALID_REQUEST` | resize | `Terminal resize failed because the request is invalid.` | Tauri の resize と同一 |
| `INVALID_REQUEST` | 不正 request | `Terminal request failed because the request is invalid.` | 接続前の JSON parse / frame kind / size と接続後の JSON parse / shape の不正を含む |
| — | ack | error response を返さない | acknowledge は失敗を返さない既存契約 |
| — | kill | WebSocket operation ではない | `kill_terminal_surface` Tauri command を使用する既存契約 |

WebSocket attach は cap 系の `UsecaseError`（`PerWorktreeCap` / `TotalCap`）を返さないため、`CAP_REACHED` mapping を持たない。`terminalStreamSocket.ts` は表の message をそのまま渡す。message がない error frame は backend error として言い換えず malformed frame として console に記録し、socket を閉じて fallback / resync を開始する。

#### `input_unavailable` の内部原因と message

| domain event の原因分類 | protocol の message | wire に含めない内部原因 |
| --- | --- | --- |
| `StaleAttachment` | `Terminal input could not be sent. Try again.` | `Terminal input attachment is no longer active` |
| `PendingCapacityExceeded` | `Terminal input could not be sent. Try again.` | `Terminal input reorder buffer is full` |
| `RuntimeWriteFailed(String)` | `Terminal input could not be sent. Try again.` | runtime gateway error の文字列 |

Tauri Channel と local API WebSocket はどちらも `TerminalSurfaceStreamItemV1::from` を通るため、同じ原因分類から同じ wire message を返す。変換時に内部原因を `operation=write_terminal_surface code=PTY_ERROR cause=...` として error log へ記録する。

### Data Model

新しい domain record、永続 record、identity、versioning は追加しない。`TerminalSurfaceInputUnavailableCause` は `StaleAttachment` / `PendingCapacityExceeded` / `RuntimeWriteFailed(String)` の3 variant を持ち、`internal_cause()` が内部原因文字列の唯一の定義元となる。runtime gateway は返却する `TerminalSurfaceGatewayError` もこの値から組み立てる。`TerminalSurfaceEvent::InputUnavailable` と `TerminalSurfaceStreamItem::InputUnavailable` は表示文言ではなくこの内部原因を保持し、`TerminalSurfaceStreamItemV1::InputUnavailable` は既存 wire shape の `message` を保持する。その他の既存 wire 型であるプレーン文字列と `{code, message}` を維持し、application lifecycle の `{type, ...variantFields}` には `message` だけを additive に追加する。frontend の共通関数は error 値、code、type を保持せず、呼び出し時に文字列だけを返す。

### Database

該当なし。

### UI/UX

AgentSession の Terminal と Workspace bottom Terminal は、Terminal surface の上に `role="alert"` / `text-destructive` の失敗表示面を持つ。他の既存エラー表示位置、レイアウト、重大度、再試行操作は変更しない。backend error の表示面は受け取った message をそのまま描画し、同じ Terminal の後続の初期化・attach または再同期が成功すると表示を消す。

- Markdown diff、review file view、workflow action、Terminal backend reject にあった frontend 接頭辞を除去する。
- Terminal 初期化・attach・再同期の区別は backend message で表す。
- Terminal 初期化失敗は xterm へ重ねて書き込まず、`role="alert"` だけに表示する。
- `AgentSessionPanel` の Terminal 失敗表示はライフサイクル操作の失敗表示と別の state で管理し、Terminal 分岐だけに表示する。
- WS stream error、stream item 適用失敗、Channel write 失敗は自動的に attachment を再同期し、成功時に alert を消す。recovery 中の stream item 適用失敗は recovery を張り直さない。
- frontend 内部例外の操作文脈は維持する。
- telemetry は `Error` の stack を維持し、message の取得だけを `getErrorMessage` に委譲する。

### Algorithm

Terminal command の `UsecaseError` 変換は、error 分類から `TerminalCommandErrorCode` enum を決め、command operation と code の exhaustive match から固定 message を選ぶ。`attach_terminal_surface` は frontend が渡す `recovery` だけを操作 discriminator に使う。`write_terminal_surface` と `resize_terminal_surface` は構造化 error へ移行せず、gateway failure と owner 変換失敗をそれぞれの操作の固定文字列へ変換する。frontend は operation、code、error shape から文言を決めない。

Terminal WebSocket は `TerminalWsError::Attach(code)` / `Write(code)` / `Resize(code)` / `InvalidRequest` の exhaustive match から固定 message を選び、内部 cause は response に含めない。`InvalidRequest` variant の code は常に `INVALID_REQUEST` であり、不正 request と `PTY_ERROR` の組み合わせは構築できない。

runtime gateway は入力不能の発生点で3 variant の `TerminalSurfaceInputUnavailableCause` を構築する。usecase は原因を加工せず運び、protocol 変換が固定 message を設定して内部原因を log に残す。原因文字列の内容による分類は行わない。

AgentSession の Provider parse と launch error 変換は、Provider 設定、session 作成、履歴再開、lifecycle 更新の operation を受け取り、operation-sensitive な3 code の message を選ぶ。Provider parse、launch、conflict にはそれぞれ専用 operation 型を使い、不可能な組み合わせを構築できない。他の code は operation に依存しない一つの mapping を再利用する。

application lifecycle の7 error enum は variant 自体を変えず、手書き `Serialize` が private wire 構造体へ変換する。これにより既存 constructor と機械可読 field を維持しながら、全 variant に `message` を追加する。

### Infra

該当なし。

## Alternatives Considered

### frontend で code から表示文言を選ぶ

採用しない。表示文言の owner が frontend にも生まれ、R-003 に反する。分類結果と操作は backend の message に反映し、frontend は構造抽出だけを行う。

### code ごとに一つの message だけを持つ

採用しない。Terminal の初期化と再同期、AgentSession の作成と履歴再開のように、同じ code でも発生した事象が異なる。operation discriminator を Rust の presentation mapping に渡し、同一 code × 同一操作を一意にする。

### coded error をプレーン文字列へ serialize する

採用しない。`STALE_REVIEW_GROUP_TARGET` を含む機械可読 code を失い、既存 serialize 契約も変わる。

### application lifecycle error の全 constructor に message field を渡す

採用しない。internally-tagged enum の unit variant に定数 field は追加できず、全 command / presenter の constructor に表示専用値を持ち回ることになる。enum の形を維持した手書き `Serialize` で command wire にだけ message を追加する。

### 利用者向け文言を domain または usecase error に持たせる

採用しない。文言は operation surface 向けの外部表現であり、domain / usecase のエラー分類へ表示都合を持ち込む。Tauri command、Terminal WebSocket、Terminal stream protocol の各 adaptor 境界に留め、`ApiErrorBody` と CLI は変更しない。

### `input_unavailable` の原因文字列を protocol 境界で分類する

採用しない。gateway 実装の文言変更が wire message の判断を暗黙に変え、未知の原因を分類できない。発生点で3 variant の domain enum に分類し、runtime write の詳細だけを variant の値として保持する。

## Cross-cutting concerns

### 互換性

`AppError` と `TerminalCommandError` の wire shape、全 code 値、application lifecycle error の全 type 値と既存 variant field、`input_unavailable` の `{type, session_key, message}`、`ApiErrorBody`、CLI、`useDiffOperations.ts` の回復分岐を維持する。application lifecycle error と `TerminalWsErrorV1` の message は additive な変更である。`write_terminal_surface` と `resize_terminal_surface` の `Result<(), String>`、`attach_terminal_surface` の `recovery` 引数、成功時の応答、stream payload は維持する。

### 可観測性と検証

- `terminal_surface/commands.rs` は固定 message へ置き換える直前に、`operation`、`code`、内部原因を記録する。上限到達と owner 変換失敗は warn、gateway failure は error とする。
- `api/terminal.rs` は WebSocket の固定 message へ置き換える直前に `operation`、`code`、内部原因を記録する。INVALID_REQUEST は warn、PTY_ERROR は error とする。
- `protocol/terminal.rs` は `input_unavailable` の固定 message へ置き換える直前に、`operation=write_terminal_surface`、`code=PTY_ERROR`、内部原因を error で記録する。
- `code/mod.rs` は stale review group の固定 message へ置き換える直前に、`STALE_REVIEW_GROUP_TARGET` と内部 `group_id` を warn で記録する。
- `provider_tui_test.rs` は全AgentSession / Provider code と、操作依存 code の文言を検証する。
- `terminal_surface/commands_test.rs` は3 code、初期化・attach・再同期の文言、write と resize の各2文言、`recovery` operation mapping、CAP_REACHED の4 operation を検証する。
- `application_operation_v1_test.rs` は tagged error の全31 variant の type / message と既存 variant field を検証する。
- `api/terminal_test.rs` は WebSocket で到達可能な7つの error operation / code の message と内部 cause の非露出を検証する。
- `protocol/terminal_test.rs` は `input_unavailable` の3原因分類が固定 message へ変換され、内部原因が wire に載らないことを検証する。
- `src/lib/errorMessage.test.ts` は coded error、プレーン文字列、`Error`、fallback を検証する。
- `src/hooks/useTerminal.test.ts` は初期化中の失敗後の完走通知、再同期成功のクリア通知、古い epoch の成功によるクリア抑止、WS stream error と stream item 適用失敗の recovery、recovery 中の連続適用失敗による再入抑止、Channel write 失敗の通知と recovery、exit 済み attach のクリアを検証する。
- `RightSidebarBottom.test.tsx` と `AgentSessionPanel.test.tsx` は Terminal alert が成功通知で消えることを検証し、後者は Terminal error が paused / archived 画面へ持ち越されないことも検証する。
- その他の component / hook tests は代表的な object reject、frontend 内部例外、Terminal 初期化・ack・再同期、WebSocket message 透過、review handoff、shutdown cursor 回復、coded / tagged error telemetry を検証する。
- `src/lib/errorMessageUsage.test.ts` は production source から共通関数の利用ファイル集合を導出して対象24ファイルと完全一致させ、`String(error)` / `String(cause)`、局所的な `Error.message` ternary、私的な同名 helper の再導入を検出する。

### Risks

- 新しい `getErrorMessage` 利用ファイルが対象24ファイルの一覧に追加されない可能性がある。production source から導出した利用ファイル集合との完全一致テストで検出する。
- frontend と backend で `recovery` の意味がずれると Terminal の事象文言が誤る。初回 attach が `false`、回復 attach が `true` を渡す hook test と、Rust の operation mapping test で固定する。
