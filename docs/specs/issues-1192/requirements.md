# Requirements

## Type

不具合修正（堅牢性改善を含む）

## Goal

Agent チャットで 1 ターンが正常完了した後、同一セッションへ 2 通目以降を送っても、エージェントの応答が正常に返ること。「一度会話が完了すると同一セッションでターンを繋げられず無反応になる」状態を解消し、同一セッション内で連続したターンの会話を継続できるようにする。

完了時には、ターン完了後に bridge プロセスが終了していても Rust 側がそれを検知し、2 通目送信時に（必要なら resume 付きで）新しい bridge プロセスを起動して応答を返せる状態になっている。

## Background

Agent チャットで以下の症状が報告されている（GitHub Issue #1192）。

- 1 ターン目はエージェントが正常に応答する。
- ターンが正常完了した後、同じセッションに 2 通目を送ると、送信自体は受理されて自分のメッセージは UI に表示されるが、エージェントの応答が一切返らず無反応になる。
- 結果として、一度会話が完了すると同一セッションでターンを繋げられない。

真因は 2 層の問題の合わせ技であり、いずれも実コードで確認済みである。

- **② Rust が完了後（state=Ready）のプロセス死亡を検知しない（確実なコード欠陥）**
  `apply_bridge_eof_crash`（`src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs:1376` 付近）は、bridge プロセスの EOF（プロセス終了）を `Crashed` 扱いするのが `state == Streaming || Initializing` のときだけである。一方、ターン正常完了時は `run_turn_complete_transition_locked`（同 `:1219` 付近）で `state = Ready` に遷移する。そのため、完了後に bridge が終了して stdout EOF が来ても何もせず、死んだプロセスをハンドルマップに残す。次に 2 通目を送ると `ensure_runtime_for_turn`（同 `:3877` 付近）の spawn 判定が `state == Crashed` のときしか再 spawn しないため、死んだプロセスを「生きている」と誤認して再 spawn せず、誰も読まない stdin に書き込むだけで無反応になる。

- **① bridge がターン完了後にプロセスを終了する**
  `src-tauri/resources/claude-sdk-bridge.mjs` は常駐型（複数ターンを 1 プロセスで処理）を意図している（generator を close コマンドが来るまで閉じない設計）。しかし外側ループの `for await (const message of currentQuery)` が result 後に正常完了（イテレータ完了）すると、`aborted` でないため `break` → `process.exit(0)` に到達し、プロセスが終了する。`@anthropic-ai/claude-agent-sdk` の `query()` イテレータが streaming-input モードで result 後にイテレータを完了させる挙動になっていると、毎ターンこの exit が発生し、「1 ターン目 OK・2 通目無反応」という症状と完全に一致する。

② は単体で確実なコード欠陥であり、これを直すだけでも 2 通目で再 spawn が走り会話は継続できる。① は bridge を常駐させ毎ターンの再 spawn を避ける改善であり、SDK の実挙動確認とセットで対応する位置づけである。

## Users / Actors

- Agent チャットで同一セッションを跨いで複数ターンの会話を行う Releash 利用者。
- Agent ターンを駆動する Rust 側の bridge ランタイム（`bridge_common.rs`）。
- ターンを処理する bridge プロセス（`claude-sdk-bridge.mjs` / node プロセス）。

## Scope

- **②** ターン正常完了後（state=Ready / turn_phase=Idle）に bridge プロセスが終了し stdout EOF となった場合に、Rust 側がそれを検知し、死んだプロセスを再利用しないようにする。
- **②** 終了済み bridge が残っている状態でも、2 通目以降の送信時に再 spawn し、（既存セッションを継続するため）resume を伴って会話を継続できるようにする（① が機能せず bridge が終了した場合のフォールバックとして堅牢性を担保する）。
- **①** bridge プロセスが result 後にイテレータ完了で正常終了してしまう挙動を修正し、設計意図どおり常駐させて毎ターンの再 spawn 自体を避ける（SDK の実挙動確認を含む）。
- 上記修正（①・②の両方）に対する正常系・異常系の自動テストを追加する。

## Non-goals

- Agent チャットの会話モデル・UI・権限/承認フローの変更。
- セッション間（異なる chat_session_id 間）でのコンテキスト共有や履歴統合。
- bridge ↔ Rust 間のプロトコル（メッセージ型）の再設計。
- ストリーミング中・初期化中のクラッシュ検知（既に動作している既存挙動）の仕様変更。
- 2 通目で再 spawn する際の resume 機構そのものの新規実装（既存の resume 経路を利用する前提。存在しない場合は別途報告）。

## Requirements

- ターンが正常完了した後、同一セッションへ 2 通目以降を送信したとき、エージェントの応答が返ること（無反応にならないこと）。
- **①** bridge プロセスは、ターンが result 後に正常完了（イテレータ完了）した場合でも `process.exit` せず、設計意図どおり常駐して次ターンを同一プロセスで処理できること（close コマンド到達時のみ終了する）。
- **②** 万一ターン完了後に bridge プロセスが終了して stdout EOF となった場合でも、Rust 側はその終了を検知し、当該プロセスを「再利用可能な生存プロセス」として扱わないこと（ハンドルマップから除去するか、再 spawn 対象の状態に倒すこと）。
- **②** 終了済み bridge が残っている状態で次ターンを開始するとき、`ensure_runtime_for_turn` 相当の判定が再 spawn を必要と判断し、新しい bridge プロセスと新しい stdout 購読タスクが起動すること。
- 再 spawn 後のターンは、既存セッションの文脈を保ったまま（resume を伴って）継続できること。
- ストリーミング中・初期化中の bridge クラッシュ検知（既存挙動）が、本修正によって退行しないこと。
- 上記の状態遷移（① 常駐継続、② 完了 → EOF 検知 → 次ターンで再 spawn）を検証する自動テストが存在すること。

## Constraints

- 既存の `apply_bridge_eof_crash` / `run_turn_complete_transition_locked` / `ensure_runtime_for_turn` の責務境界を尊重し、状態遷移の一貫性を壊さないこと。
- すべてのロジックは Rust（Tauri バックエンド）側に実装する。bridge（mjs）側の対処はプロセスのライフサイクル管理に限定する。
- 修正後も、ストリーミング中クラッシュ時のエラーメッセージ表示・streaming parts の確定（consolidate）といった既存のユーザー可視挙動を維持すること。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を満たすこと。

## 受け入れ基準の概要

- 同一セッションで 1 ターン目完了後に 2 通目を送ると、エージェントが応答する（手動再現で無反応が解消される）。
- **①** ターン正常完了後も bridge（node）プロセスが生存し続け、同一プロセスで次ターンを処理する（常駐挙動を実行ログ/手動で確認できる）。
- **②** 単体テストで、`state=Ready`（および `turn_phase=Idle`）の状態で bridge EOF が発生したとき、当該プロセスが再利用対象外（マップから除去 or 再 spawn 判定が true）になることを検証できる。
- **②** 単体テストで、終了済みプロセスが残った状態から次ターン開始時に再 spawn 判定が true になることを検証できる。
- ストリーミング中・初期化中の EOF クラッシュ検知に関する既存テストが引き続き通る。

## 仮定

- 修正対象は `feat/issues/1192` ブランチ。Spec ディレクトリは `docs/specs/issues-1192/`。
- 受け入れの最終基準は「2 通目以降も応答が返ること」という外部観測可能な振る舞いであり、①（bridge 常駐）と ②（Rust 側 EOF 検知修正）の両方を必須スコープとする（ユーザー合意済み）。① を主たる正常経路、② を ① が機能しなかった場合のフォールバック兼堅牢性担保として位置づける。
- 2 通目の再 spawn 時に既存セッションを継続するための resume 経路は、既存実装に存在する前提で利用する。
- `@anthropic-ai/claude-agent-sdk` の `query()` イテレータが result 後にイテレータを完了させるか継続するかの実挙動は、① の実装時に実行ログで確認する。確認結果に応じて mjs 側の外側ループの繋ぎ直し方法が定まる。

## Open Questions

なし
