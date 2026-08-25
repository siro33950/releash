# Context

- 変更要求の正本: [#1652 [frontend] Coded AppError がエラー表示で [object Object] になる](https://github.com/siro33950/releash/issues/1652)（label: bug / milestone なし）
- 実障害: 2026-08-19、PJT-2308 worktree で AgentSession 起動が `AGENT_SESSION_TERMINAL_UNAVAILABLE` で失敗し、WorkspaceList のエラーボックスに `[object Object]` だけが表示された。
- 表示文言の決定は Rust が所有する。frontend は backend が返した文言の構造抽出と表示だけを担い、`code` や error shape から表示文言を決めない。
- `docs/architecture/CONTROLLER.md` は Tauri command の戻り値を `Result<_, AppError>` に統一する方針を定めているが、現時点では `Result<_, String>` の command が77個残る。この移行は本変更に含めない。

# Outcome

- backend が返した利用者向け文言が、coded error でも `[object Object]` に失われず、そのまま表示・報告される。
- 利用者が Terminal の初期化、既存 Terminal への attach、稼働中 Terminal の再同期を文言だけで区別できる。
- Terminal の失敗表示は、同じ Terminal の後続の初期化・attach または再同期が成功すると消える。
- 同じ code でも発生した操作が異なる場合は、Rust の command 境界が操作に対応する固定文言を選ぶ。
- 利用者向け固定文言へ置き換えた内部原因は、code と対応づけて backend log から追跡できる。
- Terminal の入力不能通知は transport に依存しない固定文言を返し、内部原因を利用者向け wire へ露出しない。

# Current Behavior

## backend が返すエラーの形

- `AppError::Internal(String)` はプレーン文字列として serialize される。
- `AppError::Coded { code, message }` は `{code, message}` のオブジェクトとして serialize される。
- Terminal の3 command は `TerminalCommandError` を返す。wire shape は `{code, message}` で、code は `CAP_REACHED` / `PTY_ERROR` / `INVALID_REQUEST` である。
- `write_terminal_surface` と `resize_terminal_surface` は `Result<(), String>` を維持し、失敗時は Rust の command 境界で操作に対応する固定文言へ変換する。
- Terminal WebSocket の error response は `{status: "error", error: {code, message}}` で、message は操作ごとの固定した利用者向け文言である。
- Terminal attachment stream の `input_unavailable` は `{type: "input_unavailable", session_key, message}` を維持し、domain の内部原因を protocol 境界で固定した利用者向け `message` へ変換する。
- workflow、app config、review handoff、workspace tree などには `Result<_, String>` の command が残る。
- application lifecycle command は `type` discriminator と利用者向け `message` を持つ専用の tagged error を返す。`correlation_id` / `failure` など variant 固有の既存フィールドも維持する。
- `src-tauri/src/other/error.rs` の serialize 契約、code と type の値、`adaptor/controller/api/error.rs` の `ApiErrorBody`、CLI の契約は変更対象ではない。

## frontend の受け取り経路

Tauri IPC または frontend library が reject した値を利用者へ表示、上位へ返却、または telemetry として報告する共通抽出境界は次の24ファイルである。静的テストは production source から `getErrorMessage` の利用ファイル集合を導出し、この一覧との完全一致を検証する。

| ファイル | 現在受け取り得る backend error |
| --- | --- |
| `src/components/workspace/WorkspaceList.tsx` | AgentSession / Provider の `AppError`、workflow の `String` |
| `src/hooks/useProviderAvailabilitySettings.ts` | `AppError` |
| `src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx` | `AppError` |
| `src/components/panels/MarkdownDiffViewer.tsx` | `AppError` |
| `src/hooks/useReviewFileView.ts` | `AppError` |
| `src/lib/telemetry.ts` | unhandled error / rejection の任意の shape |
| `src/hooks/useTerminal.ts` | `TerminalCommandError`、`String`、frontend 内部例外 |
| `src/components/panels/NodeContentView/NodeContentView.tsx` | `String` |
| `src/hooks/useWorkspaceNodeDetail.ts` | `String` |
| `src/components/panels/automation/FacetEditor.tsx` | `String` |
| `src/components/panels/automation/NameInputDialog.tsx` | `String` |
| `src/lib/workflowExecutionActions.ts` | `String` |
| `src/hooks/useUpdateChecker.ts` | plugin 由来の error |
| `src/components/panels/SettingsModal.tsx` | repository の `AppError`、app config / editor の `String` |
| `src/hooks/useAutomation.ts` | `String` |
| `src/screens/useWorktreeGitActions.ts` | `AppError` |
| `src/hooks/useWorkflowConfig.ts` | `String` |
| `src/hooks/useAppSettings.ts` | app config の `String`、autostart plugin の error |
| `src/components/workspace/DeleteWorktreeDialog.tsx` | `AppError` |
| `src/components/panels/DiffToolbar.tsx` | `String` |
| `src/hooks/useWorkspaceTreeNodes.ts` | `String` |
| `src/contexts/ReviewThreadHandoffContext.tsx` | review handoff の `String`、clipboard の frontend 例外 |
| `src/hooks/useNotionSettings.ts` | `String` |
| `src/hooks/useApplicationShutdownSupervision.ts` | application lifecycle の tagged error |

これらの経路で `String(error)` または `error instanceof Error ? error.message : String(error)` を局所的に使うと、Tauri が reject した `{code, message}` や `{type, message}` が `[object Object]` になる。24ファイルは共通の `getErrorMessage` を使用し、production source 全体で同目的の局所抽出を再導入しない。

Terminal error は次の直接伝播経路も通る。ここでは frontend が文言を抽出・加工せず、Rust が返した message をそのまま callback と alert へ渡す。

| ファイル | 責務 |
| --- | --- |
| `src/lib/terminalStreamSocket.ts` | `TerminalWsErrorV1.message` を接頭辞なしで `useTerminal` へ渡す |
| `src/hooks/useTerminal.ts` | command / WebSocket / attachment stream の backend message を `onTerminalError` へ渡す。Channel fallback の write reject も `getErrorMessage` で抽出した文言を無加工で渡す。初期化・attach 完走時と現行 epoch の再同期成功時は `null` を渡す |
| `src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx` | AgentSession の Terminal error をライフサイクル error と分離した state で保持し、`role="alert"` で表示・クリアする |
| `src/components/panels/RightSidebarBottom.tsx` | Workspace の Terminal error を専用 state で保持し、`role="alert"` で表示・クリアする |

## frontend が文脈を付加する経路

- backend が表示文言を所有する reject には、frontend の接頭辞・接尾辞を付けない。
- Channel fallback の `write_terminal_surface` reject は backend が表示文言を所有するため、frontend は `getErrorMessage` による抽出だけを行い、操作文脈を付加しない。
- `ack_terminal_surface_output` の IPC 失敗、Terminal renderer 内部の初期化・再同期・stream 適用失敗、clipboard 書き込み失敗など、backend が利用者向け文言を所有しない frontend 側の失敗には操作文脈を維持する。
- `ReviewThreadHandoffContext.tsx` は `build_review_thread_handoff` の reject と clipboard 例外を別々に扱う。

## 既存の機械可読分岐

- `src/components/panels/useDiffOperations.ts` は `STALE_REVIEW_GROUP_TARGET` を検出して snapshot を再取得する。この回復挙動は表示文言の選択ではないため維持する。
- `src/hooks/useApplicationShutdownSupervision.ts` は pending attempt の非 null cursor に対する `type: "invalid_request"` だけを cursor 回復条件として扱う。message 文字列の部分一致は行わない。

# Scope / Non-goals

## Scope

- 上表の24ファイルを `src/lib/errorMessage.ts` の `getErrorMessage` へ統一し、production source から導出した利用ファイル集合と機械照合する。
- production source 全体から同目的の `String(error)` と `instanceof Error` による局所抽出を除去し、静的テストで再導入を検出する。
- backend 由来の表示文言に frontend が付けている接頭辞・接尾辞を除去する。
- frontend 内部例外と backend command reject が同じ catch に入る経路を分離する。
- 既存25 code の利用者向け英語文言を Rust の command 境界で確定する。
- application lifecycle の tagged error 全 variant に、既存 type と variant 固有フィールドを維持した利用者向け message を追加する。
- Terminal と AgentSession の操作依存 code は、code を変えずに操作ごとの固定文言を返す。
- Terminal WebSocket の attach / write / resize / 不正 request に、Rust が所有する操作別の固定文言を設定する。
- Tauri Channel fallback の write 失敗を Rust command 境界の固定文言として通知し、WebSocket write と同じ文言・回復処理にする。`resize_terminal_surface` も同じ command 境界で WebSocket resize と同じ固定文言を返す。
- `input_unavailable` の3内部原因を domain event で分類し、protocol 境界で transport 非依存の利用者向け固定文言へ変換する。
- Terminal の初期化・attach 完走または現行 epoch の再同期成功を表示面へ通知し、同じ Terminal の失敗表示をクリアする。
- 利用者向け固定文言への変換で失われる Terminal command の owner 変換原因、Terminal WebSocket と `input_unavailable` の内部原因、stale review group ID を、code とともに backend log へ記録する。
- backend 文言、frontend 抽出、操作別文言、回復分岐、静的検証をテストで固定する。

## Non-goals

- `Result<_, String>` を返す Tauri command 77個の `Result<_, AppError>` への移行。
- `AppError` の serialize 表現の変更。`Internal` はプレーン文字列、`Coded` は `{code, message}` を維持する。
- code の値・命名・粒度の変更、および coded error の追加・削除。
- `adaptor/controller/api/error.rs` の `ApiErrorBody` と CLI のエラー契約の変更。
- `useDiffOperations.ts` の `STALE_REVIEW_GROUP_TARGET` 回復挙動の変更。
- frontend 内部エラーの文言を Rust へ移管すること。
- Terminal の `role="alert"` 表示面以外のエラー UI のレイアウト・配置・重大度表現の変更。
- UI 文言の多言語化。
- `src/App.tsx` が `catch {}` で処理する `quit_after_startup_failure` の表示・telemetry 経路の変更。

# Requirements

- R-001: Tauri command が `{code, message}` 形式の coded error で reject したとき、利用者に表示される文言は backend が返した `message` と一致し、`[object Object]` は表示されない。
- R-002: Tauri command がプレーン文字列で reject したとき、利用者に表示される文言は reject された文字列と一致し、frontend が付加した接頭辞・接尾辞を含まない。
- R-003: frontend は backend 由来のエラーに対して、code や error shape を用いた文言選択、接頭辞・接尾辞の付加、言い換えを行わない。
- R-004: Terminal 初期化の `CAP_REACHED` とそれ以外の失敗は、backend が返す文言だけで区別できる。
- R-005: backend structured error の message は、利用者が対象、発生した事象、回復可能な場合の行動を識別できる英語の文言である。同じ code でも操作が異なる場合は、同一 code × 同一操作で一意な文言を返す。
- R-006: code は機械可読な回復分岐に引き続き利用でき、`STALE_REVIEW_GROUP_TARGET` の snapshot 再取得は変わらない。
- R-007: frontend が coded error または tagged error を telemetry として backend へ報告するとき、報告される `message` は backend が返した `message` と一致する。
- R-008: R-001、R-002、R-007 は上表の24ファイルと Terminal の直接伝播経路で成立し、production source に同目的の局所抽出が残らない。
- R-009: 利用者向け固定文言への変換で表示から除かれた Terminal command の owner 変換原因、Terminal WebSocket の内部原因、`input_unavailable` の内部原因は `operation` / `code` / `cause` として、その他の内部原因は code と原因を識別できる文脈とともに backend log に記録され、利用者向け error と内部原因を突き合わせられる。
- R-010: application lifecycle command の tagged error は全 variant が利用者向け `message` を持ち、既存の `type` と variant 固有フィールドを維持し、表示・報告で `[object Object]` にならない。
- R-011: Terminal WebSocket error は操作に対応する Rust 所有の固定 message を返し、Tauri command と同じ attach、write、resize の失敗は transport に依存せず同じ message になる。
- R-012: Terminal の失敗文言が表示されているとき、同じ Terminal の後続の初期化・attach 完走または現行 attachment epoch の再同期成功により失敗表示はクリアされ、古い epoch の成功は現行の失敗表示をクリアしない。
- R-013: `input_unavailable` は stale attachment、pending capacity 超過、runtime write 失敗を domain event で分類し、protocol 境界で利用者向け固定 message へ変換する。wire に内部原因を含めず、Tauri Channel と local API WebSocket で同じ message を返す。

# Assumptions / Open Questions

## Assumptions

なし。

## Open Questions

なし。
