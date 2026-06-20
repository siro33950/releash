# Design

## Source
- requirements.md
- behavior.md

## 概要

Agent チャットで「1 ターン目は応答するが、ターン正常完了後に同一セッションへ 2 通目を送ると無反応になる」不具合（GitHub Issue #1192）を解消する。真因は 2 層の合わせ技であり、両方を修正する。

- **②（Rust 側 / 確実なコード欠陥）** ターン正常完了で `state = Ready` に遷移した後に bridge プロセスが終了し stdout EOF が来ても、`apply_bridge_eof_crash` が何もせず、死んだプロセスをハンドルマップに残す。次の `ensure_runtime_for_turn` は `state == Crashed` のときしか再 spawn しないため、死んだプロセスを生存とみなし stdin に書くだけで無反応になる。→ 完了後 EOF を検知し、当該ハンドルを再利用対象外にする。
- **①（bridge mjs 側 / 常駐化）** `claude-sdk-bridge.mjs` は常駐型（複数ターンを 1 プロセス）を意図しているが、`for await (const message of currentQuery)` が result 後にイテレータ完了すると `break → process.exit(0)` に到達し、毎ターンプロセスが終了する。→ close コマンド到達時以外は exit せず常駐させ、毎ターンの再 spawn 自体を避ける。

①を主たる正常経路、②を①が機能しなかった場合のフォールバック兼堅牢性担保として位置づける（requirements 仮定と一致）。最終的な受け入れ基準は「2 通目以降も応答が返る」という外部観測可能な振る舞い。

## 変更対象

| ファイル | 変更内容 |
|---|---|
| `src-tauri/resources/claude-sdk-bridge.mjs` | ①: result 後のイテレータ完了でプロセスを終了させず、close まで常駐させる |
| `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` | ②: `apply_bridge_eof_crash` の完了後 EOF 検知、EOF 後の runtime retire、次ターン開始時の再 spawn 判定 |
| 同上（`#[cfg(test)]`） | ①②の状態遷移を検証する単体テスト追加 |
| `src-tauri/resources/*.test.mjs`（新規または既存に追記） | ①の常駐挙動を検証するテスト（後述の制約あり） |

プロトコルメッセージ型・UI・権限フロー・resume 機構そのものは変更しない（Non-goals）。

## アーキテクチャと責務分割

修正は既存の責務境界を尊重し、状態遷移の一貫性を壊さない（Constraints）。

- **`apply_bridge_eof_crash`（純粋関数）** — bridge EOF 時の状態判定とエフェクト生成。既存の「Streaming / Initializing → Crashed＋エラー UI 表示」の責務は不変のまま、新たに「Ready（turn_phase=Idle）での EOF＝正常完了後の終了」を識別し、エラー UI を出さずに「runtime retire が必要」を示すエフェクトを返す責務を追加する。
- **EOF 呼び出し元（stdout reader）** — `apply_bridge_eof_crash` のエフェクトに従い、session runtime lock を取ったうえでハンドルマップを操作する。新たに「runtime retire が必要」なエフェクトを受けたら `retire_ready_eof_runtime_locked` に委譲し、pending queue が空なら当該 chat_session_id のハンドルを除去し、pending queue が残っていれば queue を保持したまま `BridgeState::Crashed` / `TurnPhase::Idle` の非ユーザー可視 marker として残す。
- **`ensure_runtime_for_turn`（再 spawn 判定）** — `take_runtime_requiring_spawn_locked` に判定を集約する。ハンドルが無ければ `Missing → 再 spawn`、`Crashed` marker または既存 crashed runtime は `Replace → pending_messages を退避して再 spawn`、さらに `Ready` / `Idle` の child が stdout EOF 処理より先に終了済みなら `try_wait` で検知して同じ `Replace` 経路に倒す。spawn guard 取得後にも同じ判定を再実行し、競合時も pending queue を新 runtime へ prepend する。再 spawn 時の resume 情報（`agent_session_id`）はターン成功時に SessionStore へ永続化済みで、ハンドル除去または marker 化の影響を受けないため、既存 resume 経路で文脈が継続する。
- **`claude-sdk-bridge.mjs`（プロセスライフサイクル）** — ロジックは持たず、プロセスを設計意図どおり常駐させるライフサイクル管理に限定する（rust-first-logic に従い、対処範囲をライフサイクルに限定）。

### ②の runtime retire と `Crashed` marker（設計判断）

requirements は「ハンドルマップから除去する **または** 再 spawn 対象の状態に倒す」の両方を許容している。本設計では pending queue の有無で分岐する **runtime retire** を採用する。

- pending queue が空ならハンドルを除去する。次ターンは `ensure_runtime_for_turn` の `Missing` 経路で素直に再 spawn され、エラー状態通知（`SessionState::Error`）を経由しない。
- pending queue が残っている場合はハンドルを即除去しない。死んだ runtime を `BridgeState::Crashed` / `TurnPhase::Idle` の marker として残し、次の `ensure_runtime_for_turn` の `Replace` 経路で pending queue を退避して新 runtime へ引き継ぐ。
- この marker は「ユーザーに見せるクラッシュ」ではなく、pending queue を保持するための非ユーザー可視の再 spawn marker である。Streaming / Initializing の EOF で行うエラー UI 表示・`SessionState::Error` 通知とは分離する。

これにより、正常完了後 EOF をエラーとして扱わずに再 spawn 可能にしつつ、EOF と次ターン送信が近接した場合の queued message を失わない。

## データモデルまたは型

`BridgeEofCrashEffect` にフィールド `should_evict` を 1 つ追加する。

```rust
#[derive(Debug, Default)]
struct BridgeEofCrashEffect {
    was_streaming: bool,
    was_initializing: bool,
    message_id: Option<String>,
    error_delta: Vec<MessagePart>,
    persisted_parts: Vec<MessagePart>,
    sdk_error_message: Option<String>,
    should_evict: bool, // 追加: Ready(Idle)での EOF = 正常完了後の終了。エラー UI 無しで runtime retire
}
```

- `should_evict` は `generation_matches && *state == BridgeState::Ready && *turn_phase == TurnPhase::Idle` のとき `true`。
- 既存の `BridgeState`（`Initializing/Ready/Streaming/Crashed`）・`TurnPhase` の enum は変更しない（新バリアントは追加しない）。
- `retire_ready_eof_runtime_locked(map, chat_session_id)` は `should_evict` を受けた呼び出し元が使う map 操作用ヘルパーである。pending queue が空なら `map.remove`、pending queue が残っていれば runtime を `Crashed` / `Idle` marker として残す。
- `take_runtime_requiring_spawn_locked(map, chat_session_id)` は `Missing` / `Replace(AgentProcess)` / `Reuse` を返す再 spawn 判定ヘルパーである。`Crashed` marker と `Ready` / `Idle` child 終了の両方を `Replace` として扱い、呼び出し元が pending queue を新 runtime へ引き継げるようにする。

## 処理フロー

### ②: 完了後 EOF 検知 → 次ターンで再 spawn

```
ターン正常完了
  run_turn_complete_transition_locked → state=Ready, turn_phase=Idle
bridge プロセス終了（① が機能しなかった場合）
  stdout EOF
  apply_bridge_eof_crash(generation_matches, state=Ready, ...)
    → was_streaming=false, was_initializing=false（既存どおりエラー UI 無し）
    → should_evict=true
  EOF 呼び出し元: should_evict が true なら retire_ready_eof_runtime_locked
    → pending queue なし: map.remove(chat_session_id)
    → pending queue あり: state=Crashed, turn_phase=Idle の marker として残す
2 通目送信
  ensure_runtime_for_turn: take_runtime_requiring_spawn_locked
    → Missing または Replace(Crashed marker) → 再 spawn
    → Replace の pending_messages を新 runtime へ prepend
  spawn_runtime() が SessionStore の agent_session_id を resume として使用
  → 既存セッションの文脈を保ったまま新プロセスで応答
```

runtime retire は `generation_matches`（`proc.generation_id == captured_gen_id`）を前提とする。次ターンが先に新プロセスを spawn 済みなら generation 不一致で `should_evict=false` となり、新プロセスを誤って retire しない。

stdout EOF の処理より前に OS 上の child 終了だけが観測できる場合もある。その場合は `ensure_runtime_for_turn` が `Ready` / `Idle` runtime に対して `try_wait` を行い、終了済みなら `Crashed` marker に倒して `Replace` 経路に乗せる。これにより、stdout reader の EOF タスクが遅れても死んだ stdin へ書き込まない。

### ①: bridge 常駐（毎ターン再 spawn の回避）

`claude-sdk-bridge.mjs` の `handleInit` 内 `while (!closed)` ループで、`for await ... of currentQuery` が **正常完了（イテレータ完了）かつ `!closed` かつ `!aborted`** のとき、`break → process.exit(0)` させず外側ループを継続する。継続時は `currentSessionId` が設定済みなら `options.resume = currentSessionId` を用いて同一プロセス内で次ターンを処理する（既存の resume 設定ロジック `:219` を流用）。`process.exit(0)` は `closed`（close コマンド）でループを抜けたときのみ実行する。

```
turn 完了（result 受信 → turn_complete emit）
  for await ループがイテレータ完了で抜ける
  closed=false かつ aborted=false
  → break せず外側 while を継続（process.exit しない）
  → 次の query() を resume 付きで確立し、promptGenerator が次 message を待つ
close コマンド受信
  closed=true → ループ終了 → process.exit(0)
```

#### SDK 実挙動への分岐（実装時に実行ログで確認）

`@anthropic-ai/claude-agent-sdk` の `query()` イテレータが streaming-input モードで result 後にどう振る舞うかにより、繋ぎ直し方法が変わる（requirements 仮定 / behavior 仮定と整合）。実装時に実行ログで確認し、以下いずれかを採る。

- **(A) イテレータが複数ターンを跨いで継続する場合** — 既存の単一 `query()` ＋ 長命 `promptGenerator` が想定どおり機能する。`break → exit` に到達しないよう、正常完了時の分岐を「`closed` のときのみ exit」に限定する修正で足りる。
- **(B) イテレータが result ごとに完了する場合** — 外側 `while` を継続し、ターンごとに `query()` を resume 付きで張り直す。プロセスは常駐し Rust 側の再 spawn は発生しない。

どちらでも観測される振る舞い（2 通目以降の応答継続）は同一であり、behavior の Rule を満たす。

## エラー処理

- **ストリーミング中 / 初期化中の EOF（既存挙動）** — `was_streaming` / `was_initializing` 経路は変更しない。合成エラーパートの enqueue、`consolidate_parts` による確定、`agent-sdk-message` のエラー emit、`SessionState::Error` 通知、`TurnPhase::Idle(-1)` emit はすべて従来どおり（退行させない）。
- **完了後 EOF（②の対象）** — エラーではないため UI へエラーを出さない。`should_evict` を受けた呼び出し元が runtime を retire し、pending queue が空なら除去、pending queue が残っていれば `Crashed` / `Idle` marker として保持する。ユーザーには次ターンの再 spawn ＋ resume で応答が返ることで連続性が担保される。
- **再 spawn 失敗** — `ensure_runtime_for_turn` 既存の失敗処理（`spawn_runtime()` 失敗時に `handles.remove` して `Err` 返却、`:3915` 付近）をそのまま使う。
- **mjs 側 abort / init error** — 既存の abort 継続（`continue`）・init error 時の `clear_session_id` 経路（`:2867` 付近）は変更しない。

## テスト方針

配置・方針は `src-tauri/CLAUDE.md` のテスト規約に従う。

### ②（Rust 単体テスト、`bridge_common.rs` の `#[cfg(test)]`）

1. **`apply_bridge_eof_crash` の完了後 EOF**: `state=Ready`、`turn_phase=Idle`、`generation_matches=true` で呼び、`should_evict=true` / `error_delta` 空 / `sdk_error_message=None` / `was_streaming=false` / `was_initializing=false` を検証。
2. **既存挙動の非退行**: `state=Streaming` および `state=Initializing` の既存テスト（`:7700`、`:7896`、`:7930` 付近）が引き続き通ること。`should_evict` は両ケースで `false`。
3. **generation 不一致**: `generation_matches=false` のとき `should_evict=false`（誤除去しない）。
4. **再 spawn 判定**: 終了済みプロセスがマップから除去された状態、pending queue を持つ `Crashed` marker の状態、既存の `Crashed` 状態で `ensure_runtime_for_turn` が再 spawn を実行し、pending queue を新 runtime へ引き継ぐことを既存ハーネスに倣って検証。
5. **stdout EOF 前の child 終了検知**: `Ready` / `Idle` の child が終了済みで map に残っている状態から、`ensure_runtime_for_turn` が `try_wait` で終了を検知して再 spawn し、pending queue を保持することを検証。

### ①（bridge 常駐）

`claude-sdk-bridge.mjs` はインポート時に `process.stdin` 購読や `process.exit` の副作用を持つため、純粋関数としての単体テストは困難。以下のいずれかで検証する（実装時に確定）。

- **(優先) ループ判定の純粋関数抽出**: 正常完了時に「exit すべきか継続すべきか」を決める判定（`closed` / `aborted` / `gotResult` を入力）を小さな関数へ切り出し、`*.test.mjs`（vitest、既存 `bridge-utils.test.mjs` と同方式）で網羅テストする。
- **(補完) 統合的確認**: bridge を子プロセスで起動し 2 メッセージを送って 2 回目の `turn_complete` が来る／プロセスが生存し続けることを確認。CI 制約上 SDK 実行を伴うため、自動化が難しい場合は実行ログによる手動確認（acceptance）で代替し、その旨を記録する。

### 受け入れ確認（手動）

同一セッションで 1 ターン目完了後に 2 通目を送りエージェントが応答すること、ターン完了後も node プロセスが生存し続けること（①）を実行ログ / 手動で確認する。

## リスクと代替案

- **②の除去 vs `Crashed` marker**: 採用案は pending queue の有無で除去と marker 化を切り替える hybrid retire。常に除去すると EOF と次ターン送信が近接したときに pending queue を失うリスクがある。常に `Crashed` marker に倒すと不要な死 runtime が map に残る。hybrid により、queue なしは即除去、queue ありは非ユーザー可視 marker として保持し、次の `ensure_runtime_for_turn` で必ず置換する。
- **①の SDK 挙動依存**: query() イテレータの完了タイミングは SDK 実装に依存し、バージョン更新で変わりうる。②（Rust 側 EOF 検知）をフォールバックとして必ず実装することで、①が将来退行しても 2 通目応答が担保される（多層防御）。
- **race（retire と再 spawn の競合）**: `generation_id` 比較、session runtime lock、`acquire_spawn_session_guard`、spawn guard 後の再判定により、retire と次ターン spawn の競合で生存プロセスを誤除去せず、pending queue も新 runtime へ引き継ぐことを担保する。
- **resume 経路の前提**: 再 spawn 時の文脈継続は SessionStore の `agent_session_id` → `resolve_spawn_info` → init cmd の `sessionId` → `options.resume` という既存経路に依存する。この経路が存在しない／壊れている場合は別途報告（requirements Non-goals: resume 機構の新規実装は対象外）。実コード上は `:2700`・`:3514` 付近で確認済み。

## 仮定

- 修正対象ブランチは `feat/issues/1192`、Spec ディレクトリは `docs/specs/issues-1192/`。
- ①②の両方を必須スコープとし、①を主正常経路・②をフォールバック兼堅牢性担保とする（ユーザー合意済み）。
- 2 通目再 spawn 時の resume は既存経路（永続化された `agent_session_id`）を利用する前提。実コードで存在を確認済み。
- `query()` イテレータの result 後の挙動（継続 / 完了）は実装時に実行ログで確認し、(A)/(B) いずれかの繋ぎ直しを採る。観測される振る舞いはいずれも同一。
- ②のエフェクト追加と runtime retire / 再 spawn 判定ヘルパーは内部状態（`BridgeEofCrashEffect` / `AgentProcessMap`）に限定し、bridge ↔ Rust 間のプロトコルメッセージ型は変更しない（Non-goals）。

## Open Questions

なし
