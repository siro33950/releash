# Design

関連: #1252

requirements.md / behavior.md（いずれも案 A: origin に依らず permission timeout を全面撤去）を実装方針へ落とし込む。

## 概要

Agent session ランタイムの watchdog から「権限承認待ち（`TurnPhase::WaitingPermission`）のタイムアウト打ち切り」を撤去する。`evaluate_turn_liveness` の `WaitingPermission` 分岐から permission timeout 判定を削除し、承認待ちは経過時間に関わらず `None`（＝中断しない）を返すようにする。

permission timeout は現状 `TurnOrigin::Headless`（= workflow step session）でのみ発火し、その判定のためだけに `TurnOrigin` の区別が存在する。判定撤去によって `TurnOrigin` 系の仕組み（enum・フィールド・helper・引数経路）はすべて未使用になるため、デッドコードを残さない方針（requirements 受け入れ基準）に従い連鎖的に撤去する。

Streaming 中の stale 検知（`STALE_TIMEOUT_SECS` / `TurnLivenessTimeout::Stale`）は別役割のため一切変更しない。

## 変更対象

すべて単一ファイル `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` 内で完結する。他ファイルからの参照は存在しない（`grep` で確認済み: permission timeout / `TurnOrigin` 系シンボルの参照は本ファイルのみ）。

撤去・変更する主なシンボルと位置（行は調査時点の目安）:

| シンボル | 現状 | 変更 |
|---|---|---|
| `PERMISSION_TIMEOUT_SECS`（444） | 承認待ち打ち切り閾値 const | 削除 |
| `PERMISSION_TIMEOUT_ERROR_MESSAGE`（448） | 中断時エラー文言 const | 削除 |
| `TurnLivenessTimeout::PermissionTimeout`（468） | liveness 判定バリアント | 削除（`user_message` の対応 arm も削除） |
| `TurnOrigin` enum（71-76） | `Desktop` / `Headless` | 削除 |
| `TurnOrigin::permission_timeout_applies`（78-82） | Headless 判定 | 削除 |
| `turn_origin_for_session`（84-90） | flag→origin 変換 | 削除 |
| `turn_origin_for_chat_session`（5463-5473） | session meta→origin 解決 | 削除 |
| `AgentProcess.turn_origin` フィールド（173） | ターン origin 保持 | 削除 |
| `evaluate_turn_liveness`（480-499） | `turn_origin` 引数で分岐 | `turn_origin` 引数を削除し `WaitingPermission` を常に `None` に |
| `begin_turn_liveness(origin)`（602-608） | origin を受けて格納 | `origin` 引数を削除 |
| `start_agent_turn` / `_locked` / `_with_runtime_spawner` / `_locked`（5476-5752） | `origin` を生成・スレッド | `origin` 生成と引数受け渡しを削除 |
| `turn_watchdog_decision`（4084-4106） | `proc.turn_origin` を渡す | `turn_origin` 引数渡しを削除 |
| `AgentProcess` 構造体リテラル（218, 3487, 6803, 13281, 16231, 16943 等） | `turn_origin: TurnOrigin::Desktop` | 当該行を削除 |
| 既存テスト（10383-10570 周辺ほか） | `TurnOrigin` を参照 | 後述の通り更新・削除 |

変更しないもの（非スコープ）: `STALE_TIMEOUT_SECS`、`TurnLivenessTimeout::Stale`、`STALE_ERROR_MESSAGE`、`WATCHDOG_TICK_SECS`、`turn_phase_since` フィールドそのもの、`ChatSession.workflow_step_session`（session meta 側のフィールドは他用途のため残す）。

## アーキテクチャと責務分割

レイヤー的にはすべて `infrastructure/agent_session/runtime` 内の ランタイム watchdog ロジックであり、ドメイン／ユースケース層の変更は不要。Tauri コマンド・プロトコル・フロントエンドへの影響はない（liveness 判定は完全にバックエンド内部の関心事で、外部公開 API のシグネチャに `TurnOrigin` は出ていない）。

責務の流れ（変更後）:

- `evaluate_turn_liveness(turn_phase, last_progress_at, turn_phase_since, now)` — 純関数。`Streaming` のみ stale 判定し、`WaitingPermission` / `Idle` は常に `None`。
- `turn_watchdog_decision` — `proc` の状態から上記を呼ぶ。origin を一切参照しない。
- `start_agent_turn*` 経路 — origin の解決・受け渡しを行わず、`begin_turn_liveness()` を引数なしで呼ぶ。

## データモデルまたは型

- `TurnPhase`（`Idle` / `Streaming` / `WaitingPermission`）は変更しない。`WaitingPermission` 状態自体は SDK の `canUseTool` 待ちを表す正当な状態として残る。watchdog がそれを打ち切らなくなるだけ。
- `TurnLivenessTimeout` は `Stale` のみの単一バリアント enum になる。

  - 単一バリアントになっても enum 形のまま残す（`turn_watchdog_decision` の `Timeout(TurnLivenessTimeout)` / `finalize_turn_as_timeout_locked` 等が値として扱っており、将来の timeout 種別追加余地もあるため）。`#[derive(Debug, Clone, Copy, PartialEq, Eq)]` と `user_message` は維持し、`Stale` arm のみ残す。
- `TurnOrigin` 型は完全に削除する（`Default` 実装含む）。

## 処理フロー

変更後の `evaluate_turn_liveness`:

```rust
fn evaluate_turn_liveness(
    turn_phase: TurnPhase,
    last_progress_at: Option<Instant>,
    turn_phase_since: Instant,
    now: Instant,
) -> Option<TurnLivenessTimeout> {
    match turn_phase {
        TurnPhase::Streaming => {
            let base = last_progress_at.unwrap_or(turn_phase_since);
            (now.duration_since(base) > Duration::from_secs(STALE_TIMEOUT_SECS))
                .then_some(TurnLivenessTimeout::Stale)
        }
        TurnPhase::Idle | TurnPhase::WaitingPermission => None,
    }
}
```

watchdog（`WATCHDOG_TICK_SECS = 5` ごと）は従来どおり tick するが、`WaitingPermission` では常に `Continue` を返し続け、承認・拒否・別要因（generation/turn_seq 変化、`Crashed`）でループを抜けるまで中断しない。

`start_agent_turn` 系は `turn_origin_for_chat_session(...)` の呼び出しと `origin` 引数の受け渡しを削除し、`proc.begin_turn_liveness()` を引数なしで呼ぶ。`begin_turn_liveness` は `turn_seq` インクリメント・`last_progress_at` / `turn_phase_since` 更新のみ行う（`turn_origin` 代入を削除）。

## エラー処理

- 撤去対象は「正常待機を誤って中断していた」経路のため、新たなエラー経路は発生しない。
- `PERMISSION_TIMEOUT_ERROR_MESSAGE`（「権限承認の待機がタイムアウトしたため中断しました。」）は表示されなくなる。
- stale 経路のエラー（`STALE_ERROR_MESSAGE`）と finalize 処理（`finalize_turn_as_timeout_locked`）は不変。`TurnLivenessTimeout::Stale` のみを扱うようになる。

## テスト方針

`#[cfg(test)] mod tests`（同ファイル内）を更新する。

更新:
- `liveness_marks_streaming_stale_after_last_progress_timeout` / `liveness_keeps_streaming_alive_when_progress_is_recent` — `evaluate_turn_liveness` の引数から `TurnOrigin::Desktop` を除去（挙動アサーションは不変）。
- `begin_turn_liveness(TurnOrigin::Desktop)` を呼ぶテスト（`timeout_finalize_*` / `finalize_timeout_*` / `late_turn_complete_*` ほか）— 引数なし呼び出しに修正。

削除:
- `liveness_permission_timeout_applies_only_to_headless` — 撤去対象の挙動を検証するテストのため削除。
- `turn_origin_is_derived_from_session_workflow_step_flag` — `turn_origin_for_session` 削除に伴い削除。

追加（behavior.md の受け入れ基準を担保）:
- `WaitingPermission` で `PERMISSION_TIMEOUT_SECS`（=300）相当を大幅に超える経過（例: `turn_phase_since = now - 3600s`）でも `evaluate_turn_liveness` が `None` を返すことを検証するテスト。撤去後の閾値定数に依存しないよう、十分大きい固定秒数（例: `3600`）を直接用いる。
- behavior「Streaming 維持」「Idle 無中断」に対応する既存テスト（stale 系・idle 系）が継続して通ることを確認する（必要なら明示テストを補う）。

既存の Streaming stale テスト群は stale 維持の受け入れ基準をカバーするため、撤去後もそのまま意味を持つ。

検証コマンド（`src-tauri/` で実行）:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`（デッドコード残存ゼロをここで担保）
- `cargo test`

## リスクと代替案

- リスク: 単一バリアント enum `TurnLivenessTimeout` に対し clippy が改善提案を出す可能性。`#[allow]` ではなく、`Timeout(TurnLivenessTimeout)` の保持型として実害なく残せるため許容する。問題が出れば該当箇所のみ最小対応する（スコープ内）。
- リスク: `turn_origin` フィールド削除に伴う `AgentProcess` 構造体リテラルの修正漏れ。コンパイルエラーで即検出されるため低リスク。`grep "turn_origin"` で全箇所を網羅して潰す。
- 代替案 B（Headless のみ打ち切り維持）: requirements で「承認者不在の独立 headless 実行経路は現状存在しない」「origin による分岐を設けない」と確定済みのため不採用。
- 代替案 C（閾値を極端に大きくするだけ）: デッドコードを残し「無限待機」という要求と不一致のため不採用。

## 仮定

- `TurnOrigin` は permission timeout 判定専用であり、撤去後に他用途がない（調査で確認: 参照は `evaluate_turn_liveness` / `begin_turn_liveness` / 構造体初期化 / テストのみ）。
- `TurnLivenessTimeout` は enum 形を維持する（単一バリアント化しても型は残す）。これは finalize 経路の API 形状を変えない最小変更とするため。
- 本変更は単一ファイルに閉じ、クリーンアーキテクチャ上のレイヤー移行は伴わない（既存配置のまま修正）。

## Open Questions

なし。
