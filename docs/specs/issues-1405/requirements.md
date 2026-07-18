# Requirements

## Type

不具合修正 / ライフサイクル信頼性保証（milestone 84「Agentチャット安定化」／ Phase 0 / L4 #1405）。

close / backend 切替 / アプリ終了がターン進行中でも streaming を flush せず finalize もせず、backend の終了イベントも捨てているために、再オープン時にストリーミング本文が途切れ、ツール実行が結果不明のまま残り、permission カードが永久に確認待ちのまま残る問題（監査 RT-1、重大度 high）を解消する。

関連: milestone 84 / 解消する問題: **RT-1** / 不変条件: **I1（turn 終端保証）** / 語彙: **V-D7 の `InterruptReason::SessionClosed` 追加**（F3 additive 規約の先行部分）。

## 背景と目的

### 現状の問題（監査 RT-1）

ストリーミング中にチャットタブを閉じる・backend を切り替える・アプリを終了すると、その turn は flush も finalize もされず terminal event が永久に記録されない。結果として:

1. **本文欠落**: 最後の streaming persist（1秒間隔スナップショット）以降のストリーミング本文と未 persist の pending parts がメッセージストアに書かれず消える。close は force persist しない。
2. **terminal event 欠落**: event log には `TurnStarted` と durable part event（ToolCallStarted / PermissionRequested 等）と `SessionClosed` は残るが、その turn の terminal event（`TurnCompleted` / `TurnInterrupted`）は永久に記録されず、後から修復する経路も存在しない。
3. **permission 残骸**: pending Permission part は `Pending` のまま永続化され、再オープン時に操作可能な permission ダイアログとして描画されるが、`respond_permission` は live runtime を必須とするため必ず失敗し、永久に解決不能になる。
4. **ツール実行残骸**: background Task group が `isRunning = !isCompleted` により永久スピナーのまま残る。

### 根本原因（現行コードの事実）

- `AgentSessionRuntimeUsecase::close_session`（`runtime/usecase.rs:839`）は sessions map から state を remove してから `runtime.close()` するだけで、`flush_streaming_update(force_persist=true)` / `complete_turn` / `finalize_turn` を一切呼ばない。
- state を先に消すため、close と競合する in-flight のランタイムイベントは `apply_runtime_event` の guard（`sessions.get→None`）で破棄される。
- さらに Claude / Codex の infrastructure 層は closed フラグにより shutdown 起因の終了イベント（`TurnCompleted(Interrupted)` / `Crash`）の emit 自体を抑止する（`claude/session.rs`、`codex/session.rs`）。
- `close_all`（アプリ終了: `application_lifecycle.rs`）と `set_session_backend`（`runtime/usecase.rs:801`）も同じ経路。frontend の tab close（`useAgentChat.ts`）も interrupt せず直接 `close_session` を呼ぶ。
- `finalize_turn` の本番呼び出しは `complete_turn` 経由の1箇所のみで、閉じられた turn を後から修復する経路は存在しない。

### 目的

close / backend 切替 / アプリ終了のどの経路でも、進行中の turn を「streaming flush 強制 → `SessionClosed` 理由で finalize → runtime close」の順で必ず終端させ、再オープン後に本文が flush 済みまで残り、中断チップが表示され、スピナー・permission 残骸が残らない状態を保証する（I1）。

## スコープ

- **終了手順の統一（I1）**: `runtime/usecase.rs` の `close_session` / `set_session_backend`（backend 切替）/ `close_all`（アプリ終了 hook）で、進行中 turn がある場合に次の順序を必ず実行する。
  1. **streaming flush 強制**: `flush_streaming_update(force_persist=true)` により最後のスナップショット以降のストリーミング本文・pending parts を durable 化する。
  2. **finalize**: `TurnResult::Interrupted { reason: SessionClosed }` で finalize する。既存 finalize 経路を用いて、未解決 permission（未送信 `Pending`）の `Cancelled(effective=false)` 畳み込み、ToolCall の `Interrupted` 化を行う。
  3. **runtime close**: 上記完了後に runtime を close し、state を remove する（state の先行 remove により in-flight event を捨てる現行順序を是正する）。
- **語彙追加（V-D7 先行 / F3 additive）**: `InterruptReason` に `SessionClosed` を追加する。domain（`domain/agent_session/entities/turn.rs`）と event log（`usecase/agent_session/event_log/events.rs`）の両 `InterruptReason` に additive で加える。
- **終了イベントの回収（drain）**: close 中に backend から届く最終イベント（result / turn completed）を、state 先行 remove と closed フラグにより捨てている経路（RT-1）を、finalize 前に短時間 drain して回収するよう修正する。
- **finalize 経路の再利用**: 閉じられた turn を修復する新規経路を作るのではなく、`close_session` / `set_session_backend` / `close_all` から既存の finalize 経路を呼べるようにする。
- **テスト**: streaming 中に close → 再オープンで「本文は flush 済みまで表示・スピナー / permission 残骸なし・中断チップ（SessionClosed）あり」をシナリオテストで固定する。アプリ終了→再起動でも同様であることを検証する。

## 非スコープ

- **crash / 強制終了時の dangling turn 回収**（RT-2、`Interrupted { reason: Crash }` での起動時 finalize）。これは別 Issue（L6 #1407 resume 回復の統一 / RT-2）に属する。本 Issue は正常な close / quit / backend 切替経路の finalize に閉じる。
- **pending queue の永続化・取消**（RT-3 / L3 #1404）。
- **ストリーミング本文の durable event 化**（RT-2 の Text/Thinking/Error を event 化する経路）。本 Issue は既存のメッセージストアへの force persist で flush を保証することに閉じ、durable event schema の拡張は行わない。
- **V-D7 の TurnResult / TurnStopReason / TurnStats の全面改訂**。本 Issue は `InterruptReason::SessionClosed` の additive 追加のみを先行し、それ以外の語彙改訂は該当 Issue（S4 #1392 等）に委ねる。
- **Agent 実行設定（mode / Goal / reasoning effort）の ack・reconciliation**（Phase 3 以降）。backend 切替（`set_session_backend`）については finalize 手順の適用に閉じ、provider 切替に伴う configuration handoff の再設計は行わない。
- **frontend の tab close UI フローの再設計**。frontend は引き続き `close_session` を invoke する薄い経路のままとし、finalize 保証は Rust 側で担保する（rust-first-logic）。

## 要求事項

- `close_session` が、進行中 turn を持つセッションに対して「streaming flush 強制 → `SessionClosed` 理由で finalize → runtime close」の順を必ず実行すること。state の先行 remove により in-flight ランタイムイベントを捨てないこと。
- `set_session_backend`（backend 切替）と `close_all`（アプリ終了 hook）も同じ finalize 手順を経ること。
- finalize により、その turn の terminal event（`TurnInterrupted` 相当、reason=SessionClosed）が event log に必ず記録されること。
- finalize により、未送信 `Pending` permission が `Cancelled(effective=false)` に畳まれ、再オープン後に操作可能な permission ダイアログとして残らないこと。write-ahead 済み `Responding` / `Resolving` の扱いは既存 finalize 経路の規約に従うこと。
- finalize により、進行中の ToolCall が `Interrupted` に畳まれ、background Task group を含めて永久スピナーが残らないこと。
- close 前に streaming flush が強制され、最後のスナップショット以降のストリーミング本文・pending parts が durable 化されること（損失窓が最大 flush 間隔を超えないこと）。
- close 中に backend から届く最終イベント（result / turn completed）を、finalize 前に短時間 drain して回収し、無言破棄しないこと。
- `InterruptReason` に `SessionClosed` が additive で追加され、既存の値・既存イベントの互換性を壊さないこと（F3 additive 規約）。
- 再オープン時に中断チップ（SessionClosed 由来）が表示されること。
- ロジックは Rust（usecase / domain）に置かれ、frontend は `close_session` を invoke するだけであること（rust-first-logic）。
- 上記が Rust のシナリオテスト（streaming 中 close → 再オープン、アプリ終了→再起動）で検証されていること。

## 受け入れ基準の概要

- streaming 中にチャットタブを閉じて再オープンしても、永久スピナー・永久確認待ち permission といった残骸が無いこと。
- 再オープン後に中断チップ（`SessionClosed`）が表示されること。
- アプリ終了→再起動でも、streaming 中の turn が finalize されており同様に残骸が無く中断チップが出ること。
- backend 切替（`set_session_backend`）経路でも進行中 turn が finalize されること。
- close 前の streaming 本文が flush 済みまで再オープン後に残っていること。
- event log に、閉じられた turn の terminal event（reason=SessionClosed）が記録されていること。
- `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 仮定

- spec ディレクトリ ID は `issues-1405`（ブランチ `feat/issues/1405` に対応）とする。
- 本 Issue の対象経路は「正常な close_session / set_session_backend / close_all（アプリ終了 hook）」であり、プロセスクラッシュ・強制終了時の起動時 dangling turn 回収（`Crash` 理由）は RT-2 / L6 #1407 の非スコープとする。両者は独立して着地させ、SessionClosed が Crash 理由を上書きしない前提を保つ（I1 の `ReconciliationRequired` を Crash 理由で上書きしないという規約と整合）。
- `set_session_backend` は現行 UI 上「空セッション限定」で主経路ではないが、監査 RT-1 が同一経路と指摘しているため、finalize 手順の適用対象に含める。provider 切替に伴う configuration handoff の再設計は行わず、finalize の適用のみとする。
- streaming flush の損失許容窓は現行の定期 flush 間隔（1秒）に準ずる。close 時は force persist によりこの窓内までの本文を確実に durable 化する（I3）。
- backend 終了イベントの drain は「短時間の有界待ち」で行い、無制限に close をブロックしない。drain の具体的な待機時間・打ち切り条件は design.md で確定する。
- `InterruptReason::SessionClosed` の追加は domain・event log の両 enum に対する additive 変更とし、永続化済み event の後方互換を壊さない（既存 event の deserialize が影響を受けない）。event log 側の永続表現の具体形は design.md / behavior.md で確定する。
- closed フラグにより backend infrastructure 層（Claude / Codex）が終了イベント emit を抑止している現行挙動を、finalize / drain と両立させる具体方式（emit 抑止の解除か、close 前 finalize による event 適用か）は design.md で確定する。本 requirements では「terminal event が必ず適用される」という性質のみを要求とする。
- frontend の tab close は現行どおり `close_session` を invoke する薄い経路のままとし、interrupt 判断・finalize は Rust 側が所有する。

## Open Questions

なし。
