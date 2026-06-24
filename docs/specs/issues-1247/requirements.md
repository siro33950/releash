# requirements — issues-1247

`AgentSessionEventLog` / Projector を導入し、Agent session の保存正典（source of truth）を定義する。

## Type

改善 / リファクタリング（内部アーキテクチャ整備、observable behavior は不変）

## Goal

Agent session の保存正典を、現在分散している `ChatSession` JSON / runtime memory（`AgentProcess` 等）/ streaming parts / workflow handoff から、単一の **durable event 列（`AgentSessionEventLog`）** へ収束させる。event 列を唯一の事実とし、message page・session status・workflow turn-complete input を **projection（read model 反映）** で導出できる構造を確立する。これにより streaming / persistence / recovery / workflow integration を同じ事実列から一貫して扱えるようにする。

完了時に成功と判断する状態:

- Agent session の durable event の最小語彙（`AgentSessionEvent`）が型として定義されている。
- 永続化すべき durable event と、replay 不要な live-only delta（text/tool-input/reasoning の逐次 delta）が明文化されて区別されている。
- 「event を append → read model へ projection する」境界が存在し、その境界テストがある。
- abort / timeout / bridge crash 時に、turn / tool call / permission が未完了状態（partial）のまま残らない finalization ルールが定義されている。
- 既存の外部から観測可能な振る舞い（UI 表示・session 一覧・streaming 体験・workflow 完了判定の結果）は変わらない。

## Background

現状の Agent session の状態は複数の場所に分散して保持されている（本リポジトリ調査による事実）:

- **永続層（JSON session store）**: `src-tauri/src/usecase/agent_session/session/store.rs` および `adaptor/gateway/agent_session/session_storage/` が、`sessions/{id}/meta.json` + `messages/{seq}.json` + `index.json` のディレクトリ構造で `ChatSession` / `ChatMessage` / `MessagePart` を保存する（#1213 で導入された summary index + message paging 構造）。
- **runtime memory**: `infrastructure/agent_session/runtime/bridge_common.rs` の `AgentProcess` が `streaming_parts: Vec<MessagePart>`・`BridgeState`・`TurnPhase` を turn 中に保持する。
- **streaming parts**: streaming は cumulative snapshot 方式。`accumulate_sdk_message()` が SDK メッセージを `MessagePart` vector に push し、per-delta では差分 slice を emit、`run_turn_complete_transition_locked()` で consolidate して最終版を永続化する。
- **workflow handoff**: turn 完了判定は `proc.state == BridgeState::Streaming`（user turn が実行されたか）を唯一の基準とし、workflow turn-complete 通知もこの runtime state で gating している。

このため「session の正典がどこにあるか」が曖昧で、永続層・runtime・read model のどれを見るべきかが処理ごとに異なる。OpenCode は durable event（`Prompted` / `Tool.Called` / `Tool.Success` / `Tool.Failed` / `Retried` / `Compaction.*` 等）と ephemeral live delta（`Text.Delta` / `Tool.Input.Delta` / `Reasoning.Delta`）を分離し、durable event を projector で `SessionTable` / `MessageTable` / `PartTable` 等の read model へ反映している（issue 参照リンク）。この境界を Releash にも導入することが本変更の主旨である。

### 改善対象の failure mode

1. abort 後に partial stream / running tool call が read model に残る。
2. UI reconnect 時に累積 parts が二重適用される。
3. workflow step の完了判定が runtime state と session JSON のどちらを見ればよいか曖昧。
4. permission / tool call / retry の履歴が read model にしか残らず、復旧時に再構築できない。

## Users / Actors

- **Releash 本体の開発者**: session 保存・streaming・recovery・workflow 統合を、分散した状態ではなく単一の event 列から扱えるようになる（直接の受益者）。
- **エンドユーザー**: 間接的受益者。abort / crash / reconnect 時の表示破綻（partial / 二重適用）が解消されることで体験が改善するが、新たに操作する UI 機能は本変更では追加しない。

## Scope

- `AgentSessionEvent`（durable event）の最小語彙を Rust 型として定義する。
- durable event と live-only delta の区別を型・ドキュメントの両面で明文化する。
- event を append し read model（message page / session status / workflow turn-complete input）へ projection する境界（projector）を定義する。
- 上記境界に対する単体／境界テストを追加する。
- abort / timeout / bridge crash 時に turn / tool call / permission を未完了で残さない finalization ルールを定義し、event 列上で表現できるようにする。
- 既存の実処理経路（`accumulate_sdk_message` / `run_turn_complete_transition_locked` / `run_bridge_error_transition_locked`）を event 列駆動へ置き換える（R7）。
- session status の projection 対象に、永続側 `SessionState` だけでなく runtime 由来の遷移状態（streaming 中・permission 待ち）も含める（R3）。
- 本変更が #1213（summary index + message paging）/ #1214（seq delta streaming）/ #1217（bridge_common.rs 分割）のどこに接続するかを設計コメントまたは docs に明記する。

## Non-goals

- streaming protocol を cumulative snapshot から seq delta protocol へ移行すること（#1214 が担当）。本変更は cumulative snapshot 前提のまま、event/delta の概念境界のみ導入する。
- `bridge_common.rs` を runtime / stream / persist / recovery へモジュール分割すること（#1217 が担当）。
- 既存の外部から観測可能な振る舞い（UI 表示内容・session 一覧 API の結果・streaming の見え方・workflow 完了の結果）を変更すること。本変更は内部の保存正典の再定義に限定する。
- workflow / session の UI 再編（#1220 / #1242）や telemetry 計装（#1209）。
- 既存永続データ（`messages/{seq}.json` 等）のフォーマット破壊的変更を伴うマイグレーション（互換 projector で吸収する。仮定を参照）。

## Requirements

### R1. durable event 語彙の定義
- `AgentSessionEvent` を、session の事実列を構成する最小語彙として定義する。
- 語彙は OpenCode の durable event を参考にしつつ、Releash の既存 `MessagePart`（`Thinking` / `Text` / `ToolUse` / `ToolResult` / `Error` / `Permission` / `TaskStatus` / `TodoListSnapshot` / `SystemNotification` / `Image` / `ImageRef`）と整合する範囲で定める。少なくとも、prompt 投入・tool 呼び出しの開始/成功/失敗・retry・turn 完了・interrupt（abort）・permission 解決を表現できること。

### R2. durable / live-only の分離
- replay 可能な durable event と、replay 不要で表示中のみ意味を持つ live-only delta（`Text.Delta` / `Tool.Input.Delta` / `Reasoning.Delta` 相当）を明確に区別する。
- どの SDK メッセージ／内部イベントが durable event になり、どれが live-only に留まるかを明文化する。

### R3. append → projection 境界
- durable event を append すると、read model（少なくとも message page・session status・workflow turn-complete input）が event 列から導出される境界を定義する。
- read model は event 列の projection であり、event 列を正典とする。
- **session status の projection 対象は、永続側の `SessionState`（Active / Idle / Done / Error / Closed / Archived）に加え、runtime 由来の遷移状態（streaming 中・permission 待ち、現状 `BridgeState` / `TurnPhase` 相当）も含める。** すなわち「session が今どの実行フェーズにあるか」を event 列から projection で導出できること。runtime memory が唯一の保持者である状態を残さない。

### R4. finalization ルール
- abort（interrupt）/ timeout / bridge crash の各ケースで、turn / 実行中 tool call / 未解決 permission を未完了状態のまま残さない終端 event を必ず付与するルールを定義する。
- このルールにより failure mode 1（partial 残存）・3（完了判定の曖昧さ）が event 列上で一意に解決されること。

### R5. 既存接続点の明記
- 本変更が #1213 / #1214 / #1217 のどこに接続・依存するかを、設計コメントまたは docs として残す。

### R6. observable behavior の保持
- 既存の外部から観測可能な振る舞いを変えない。初期実装は、既存の `ChatSession` JSON 構造へ projection する **互換 projector** でよい。

### R7. 既存処理経路の event 列駆動への置き換え
- 本変更のスコープとして、既存の実処理経路（`accumulate_sdk_message` / `run_turn_complete_transition_locked` / `run_bridge_error_transition_locked`）を event 列駆動へ置き換える。これらの経路が runtime memory を直接更新するのではなく、durable event の append を介して read model に反映される構造にする。
- これにより保存正典の一本化を、語彙・境界の定義に留めず実際の処理経路まで貫徹する。
- ただし observable behavior は不変（R6）を維持し、cumulative snapshot 前提・`bridge_common.rs` 未分割の現状コード上で成立させる（#1214 / #1217 の完了を前提にしない）。`bridge_common.rs` のモジュール分割そのものは Non-goal（#1217 が担当）であり、本変更は分割せずに経路を event 駆動へ差し替える。

## 受け入れ基準の概要

issue の受け入れ基準に対応する:

1. `AgentSessionEvent` の最小語彙が型として定義されている。（R1）
2. 「append event → project read model」の境界テストが存在する。（R3）
3. live-only delta と replay 可能な durable event の区別が明文化されている。（R2）
4. #1213 / #1214 / #1217 のどこに接続するかが設計コメントまたは docs に明記されている。（R5）
5. abort / timeout / bridge crash 時に turn / tool call / permission が未完了で残らない finalization ルールが定義されている。（R4）
6. 既存の observable behavior が変わらない。初期実装は互換 projector で可。（R6）
7. 既存の実処理経路（accumulate / turn-complete / bridge-error）が event 列駆動へ置き換えられている。（R7）
8. session status の projection に runtime 由来の遷移状態（streaming 中・permission 待ち）が含まれる。（R3）

詳細な Gherkin 形式の受け入れ基準は `behavior.md` で定義する。

## Constraints

- 全ロジックは Rust（Tauri バックエンド）側に実装する。フロントエンドはインターフェースに徹する（`.claude/rules/rust-first-logic.md`）。
- 既存の session 永続構造（#1213 で導入された `meta.json` / `messages/{seq}.json` / `index.json`）と矛盾しない。互換 projector で既存構造へ反映できること。
- #1214 / #1217 がまだ未完了（OPEN）であるため、本変更はそれらの完了を前提にしない。cumulative snapshot 前提・`bridge_common.rs` 未分割の現状コード上で成立すること。
- CI と同一のチェック（`cargo fmt --check`・`cargo clippy -- -D warnings`・`cargo test`）を通す。

## Success Criteria

- 上記受け入れ基準 1〜6 をすべて満たす。
- 新規ロジック（event 語彙・projector・finalization）に単体／境界テストがある。
- abort / crash / reconnect の各シナリオで、read model が event 列から決定的に再構築でき、partial / 二重適用が発生しないことがテストで示される。

## 仮定（Assumptions）

- **A1.** Spec ディレクトリ名は `docs/specs/issues-1247` とする（既存の `issues-1209` 等の命名規約に合わせる。`feat-issues-1213` 形式も併存するが、新しい番号付きディレクトリは `issues-NNN` 形式を採用する）。
- **A2.** 本ステップ（requirements）では、event 列を独立した永続ファイルとして持つか、既存 JSON 構造へ互換 projection するに留めるかという保存形式の選択は確定しない。issue が「初期実装は互換 projector でもよい」と許容しているため、初期実装は **互換 projector**（既存 `ChatSession` JSON 構造へ projection）を前提とし、event 列の独立永続化は将来拡張余地として扱う。具体的な保存形式は `design.md` で決める。
- **A3.** durable event の最小語彙は、OpenCode の語彙そのままではなく、Releash の既存 `MessagePart` / `BridgeState` / `TurnPhase` と整合する Releash 固有の語彙として定義する。
- **A4.** 本変更は内部リファクタリングであり、新たな Tauri コマンドや UI 要素の追加は伴わない（observable behavior 不変の制約による）。
- **A5.** 関連 issue #767 の扱いは、本変更の前提・依存には含めない（参考関連のみ）。

## Open Questions

なし（すべて解消済み）。

解消済みの決定事項:

- session status の projection 対象は、永続側 `SessionState` に加え runtime 由来の遷移状態（streaming 中・permission 待ち）も含める → R3 に反映。
- 本変更のスコープは語彙・境界・finalization の定義に留めず、既存の実処理経路（accumulate / turn-complete / bridge-error）を event 列駆動へ置き換えるところまで含める → R7 に反映。
