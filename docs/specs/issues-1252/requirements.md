# Requirements

## Type

Agent session ランタイムの挙動変更。

関連: #1252

## 背景と目的

Agent session ランタイムには「権限承認待ちのタイムアウト」が実装されている。`canUseTool` による承認待ち（`TurnPhase::WaitingPermission`）のまま一定時間（現状 5 分 = `PERMISSION_TIMEOUT_SECS = 300`）が経過すると、watchdog がターンを強制中断し、以下のエラーを表示する。

> 権限承認の待機がタイムアウトしたため中断しました。もう一度お試しください。

承認操作は人間の任意のタイミングに依存する。承認待ちは「進捗が止まったハング」ではなく「人間の入力待ち」であり、固定時間で打ち切るのは不適切である。本変更の目的は、**権限承認待ちをタイムアウトさせず無限に待機させる**ことで、人間の承認タイミングに依存する正常な待機を中断しないようにすることである。

なお、Streaming 中の応答停止検知（`STALE_TIMEOUT_SECS = 180`）は「ハング検知」という別の役割を持つため、本変更の対象外であり従来どおり維持する。

## 現状の実装

対象ファイル: `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`

- `const PERMISSION_TIMEOUT_SECS: u64 = 300;` — 承認待ちを 5 分で打ち切る閾値。
- `const PERMISSION_TIMEOUT_ERROR_MESSAGE` — 中断時のエラー文言。
- `evaluate_turn_liveness()` — `WaitingPermission` フェーズで `turn_phase_since` から `PERMISSION_TIMEOUT_SECS` 超過時に `TurnLivenessTimeout::PermissionTimeout` を返す。
- `TurnLivenessTimeout::PermissionTimeout` — liveness 判定結果のバリアント。
- `TurnOrigin::permission_timeout_applies()` — `Headless` のときのみ `true`。
- `turn_origin_for_session(workflow_step_session)` — workflow step session のとき `Headless`、それ以外は `Desktop`。
- watchdog は `WATCHDOG_TICK_SECS = 5` 秒ごとに `evaluate_turn_liveness` を評価する。

現状、permission timeout が実際に発火するのは `Headless`（= workflow step session、逐次 workflow 実行）のみであり、通常のデスクトップ agent チャット（`Desktop`）には適用されていない。

## スコープ

- `evaluate_turn_liveness` の `WaitingPermission` 分岐から permission timeout 判定を撤去し、承認待ちが経過時間によって中断されないようにする。
- permission timeout 撤去に伴い未使用となる定義（`PERMISSION_TIMEOUT_SECS` / `PERMISSION_TIMEOUT_ERROR_MESSAGE` / `TurnLivenessTimeout::PermissionTimeout` / `TurnOrigin::permission_timeout_applies()` 等）を整理する。
- `evaluate_turn_liveness` 関連の既存テストを本変更後の挙動に合わせて更新する。

## 非スコープ

- Streaming 応答停止検知（`STALE_TIMEOUT_SECS` / `TurnLivenessTimeout::Stale`）の挙動変更。従来どおり維持する。
- watchdog のティック間隔（`WATCHDOG_TICK_SECS`）の変更。
- 承認 UI／承認フロー自体の変更。
- workflow エンジンの実行モデルやスケジューリングの変更。
- 承認待ちのタイムアウトを設定値で可変にする等、新たな設定項目の追加。

## 要求事項

- 権限承認待ち（`TurnPhase::WaitingPermission`）の状態は、経過時間に関わらず watchdog によって中断されないこと。
- 「権限承認の待機がタイムアウトしたため中断しました。」のエラーが発生しないこと。
- Streaming 応答停止（stale）検知の挙動が従来どおり維持されること（`STALE_TIMEOUT_SECS` での中断と `STALE_ERROR_MESSAGE` の表示）。
- permission timeout 撤去により未使用になったコードが残存しないこと（デッドコードを残さない）。
- 上記挙動を担保するテストが存在すること。

- permission timeout の撤去は origin（`Desktop` / `Headless`）に依らず適用し、origin による permission timeout 分岐を設けないこと。`Headless` 限定で打ち切りを残すような分岐を導入しないこと。

## 受け入れ基準の概要

- `WaitingPermission` フェーズで `PERMISSION_TIMEOUT_SECS` を大幅に超過しても `evaluate_turn_liveness` が `PermissionTimeout` を返さない（中断されない）ことがテストで確認できる。
- `Streaming` フェーズで `STALE_TIMEOUT_SECS` 超過時に従来どおり `Stale` で中断されることがテストで確認できる。
- 撤去対象シンボルがコードベースから参照されておらず、`cargo clippy -- -D warnings` が通る。
- `cargo fmt --check` / `cargo test` が通る。

## 仮定

- 本変更の本質は「承認待ちを無限に待機させる」ことであり、Issue タイトル・受け入れ条件（「経過時間に関わらず中断されない」）に従い、`WaitingPermission` の permission timeout 判定を全面撤去する方針とする。
- permission timeout は現状 `Headless` のみで発火するため、撤去によって挙動が実際に変わるのは workflow step session である。`Desktop` は元々適用外のため挙動は変わらない。
- `TurnOrigin` 自体（`Desktop` / `Headless` の区別）は permission timeout 以外で用途がなくなる場合、未使用整理の対象になりうる。`turn_origin_for_session` / `turn_origin_for_chat_session` を含め、実際の参照状況は design.md で精査する。
- 「Headless で承認者不在の永久滞留」は現アーキテクチャでは実体のないシナリオである。コード調査により以下を確認済み:
  - `TurnOrigin::Headless` は `turn_origin_for_session(workflow_step_session)` で `workflow_step_session == true` のときにのみ設定される。
  - workflow step session は workflow engine（`create_step_session_from_resolved_settings`、`workflow_step_session: true`）のみが生成し、`create_step_session_with_settings(app: &tauri::AppHandle<R>, ...)` 経由でデスクトップ Tauri アプリのバックエンドプロセス内で実行される。
  - `releash` CLI の `workflow` サブコマンドは read-only 観測のみで workflow を実行せず、CLI 経由の headless 実行経路は存在しない。
  - したがって Headless Session は常にデスクトップ利用者の手元で動き、その利用者が承認 UI から承認できる。承認者不在の独立した headless/server 実行経路は現状存在しない。
  - 以上より、permission timeout は Headless 限定の打ち切りを残す必要はなく、origin に依らず全面撤去する（案 A）。

## Open Questions

なし。
