# Design

`requirements.md` / `behavior.md` を満たす C1（ターン完了後の `streaming_parts` 常駐メモリ解放）の実装方針を、実コードに基づいて確定する。

対象 Issue: #1194（#1191 から分離した B群 C1）。

## 概要

`AgentProcess.streaming_parts: Vec<MessagePart>` は 1 ターン分のストリーミング parts を累積するバッファであり、ターン完了時に解放されない。ターン完了処理（`run_turn_complete_transition_locked`）は確定 parts のスナップショット（`final_parts = consolidate_parts_from_slice(&proc.streaming_parts)`）を取るだけで `streaming_parts` 自体は保持し続ける。`AgentProcessMap` からプロセスが除去されるのは明示 close 時のみのため、次ターンが来ないアイドル session では完了ターン分の parts が常駐し、会話・session 数の増加に伴って累積する（R1 の常駐経路）。

一方、ターン完了後に到着する **post-turn イベント**（バックグラウンドタスクのツール完了・status 等）は、同じ `streaming_parts` へ追記し、`consolidate_parts_from_slice(&proc.streaming_parts)` で**メッセージ全体**を再確定して emit／persist する。emit（`agent-streaming-updated` / WS `AgentStreamSync`）も persist（`persist_streaming_parts` の `msg.parts = parts.to_vec()`）も **累積 parts 全体で置き換える**契約のため、post-turn の正しい表示・永続化には「完了ターン分の全 parts（base）」が必要になる。これが「完了時に解放したいバッファ」と「post-turn で必要な base」の緊張関係であり、本設計の核心。

### 採用方針（要約）

1. **ターン完了時のバッファ解放はガード条件付きにする**: `final_parts` をスナップショットした直後、`exit_code != 0 || pending_stream_part_count == 0` の場合だけ `streaming_parts` と coalescing カウンタをクリアする（`last_message_id` / `task_id_map` は post-turn のため保持）。正常完了かつ `pending_stream_part_count > 0` の場合は、pending 分の payload 確定に必要な parts を一時保持し、post-turn 処理で解放する。
2. **post-turn の base は永続ストアから遅延再構築する**: post-turn イベントを処理する際、`streaming_parts` が空であれば、`last_message_id` に対応する**永続化済みメッセージの `parts`** を `streaming_parts` へ再シードしてから `accumulate_sdk_message` する。これにより以降の emit／persist は従来どおり「累積全体」を生成でき、外部振る舞いは不変（R2/R3）。
3. **post-turn 処理後に再びバッファを解放する**: 各 post-turn delta の emit／persist 用 payload を確定したのち `streaming_parts` を再クリアする。次の post-turn イベントは（直前 delta を含む）最新の永続メッセージから再シードするため、複数 post-turn が連続しても累積は保持されず、かつ内容は一致する。R1 の「完了ターン分の parts が常駐し続けない」は、すべてのターン完了直後ではなく、即時解放条件を満たす完了直後、または post-turn 処理完了後から次ターンまでの idle 期間で成立する。

この設計は「**完了ターン分の parts をメモリに常駐させない（実解放）**」と「**emit/persist/履歴復元の外部振る舞い不変**」を同時に満たす。正常完了かつ `pending_stream_part_count > 0` のターンだけは parts を post-turn 処理まで残すが、payload 確定後に解放するため、次ターンまでの idle 期間に完了ターン分の parts が常駐し続ける状態は残らない。retain（メモリ保持）系の代替は「§リスクと代替案」で却下理由を述べる。

## 変更対象

すべて `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs` 内。フロントエンド・他レイヤー・プロトコルは変更しない（R4）。

| 箇所 | 変更 |
|---|---|
| `run_turn_complete_transition_locked`（L1237 付近） | `final_parts` スナップショット後、`exit_code != 0 || pending_stream_part_count == 0` の場合だけ完了ターン分バッファを解放（新ヘルパー呼び出し）。正常完了かつ `pending_stream_part_count > 0` の場合は post-turn 処理まで保持 |
| 新ヘルパー `release_completed_turn_streaming_buffer`（追加） | `streaming_parts` と coalescing カウンタをクリア。`last_message_id` / `task_id_map` は保持 |
| 新ヘルパー `reseed_post_turn_base_from_store`（追加） | 永続メッセージ parts を `streaming_parts` へ再シード（base 再構築） |
| stdout reader の post-turn 分岐（`_ =>`、L3054-3137 付近） | post-turn かつ `streaming_parts` 空なら base 再シード → accumulate。payload 確定後に再クリア |
| `handle_external_bridge_message` の post-turn 分岐（`_ =>`、L5922-5987 付近、Codex 経路） | 上と対称に base 再シード → accumulate → 再クリア |

> Codex backend は `handle_external_bridge_message`（app-server external bridge）経由、Claude/legacy backend は `spawn_bridge_process` の stdout reader 経由で同じ post-turn ロジックを通る。両経路に対称適用する（本 Issue の主対象は Codex だが、共有経路の是正として両方を直す）。

## アーキテクチャと責務分割

- 変更は infrastructure 層のランタイム（bridge_common）に閉じる。ドメイン／ユースケース／プロトコル／フロントには波及させない（R4、クリーンアーキテクチャ非改変）。
- ターン完了の状態遷移は引き続き共有ヘルパー `run_turn_complete_transition_locked` に集約し、stdout reader と external handler、および単体テストが同一コードパスを通る設計を維持する。バッファ解放もこのヘルパー内に置き、両経路で一貫させる。
- post-turn の base 再構築は専用ヘルパーに切り出し、stdout reader と external handler から共通利用する（ロジック重複を避ける）。
- 永続ストア（`SessionStore`）は既に post-turn persist で都度 `get_session` + `save_session` する経路。base 再シードの読み出しも同じ I/O クラスに収まり、新たな依存方向違反を生まない。

### 解放ヘルパー（責務）

```rust
/// ターン完了時に「完了ターン分」の累積バッファを解放する。
/// final_parts スナップショット後に呼ぶこと。post-turn 追記に必要な
/// last_message_id / task_id_map は保持する（reset_streaming_state_for_new_turn
/// とは異なり、次ターン用の全リセットではない）。
fn release_completed_turn_streaming_buffer(proc: &mut AgentProcess) {
    proc.streaming_parts.clear();
    proc.streaming_parts.shrink_to_fit(); // 容量も返却し常駐量を実減少させる
    proc.pending_stream_part_count = 0;
    proc.pending_stream_bytes = 0;
    proc.last_stream_emit_at = None;
}
```

`reset_streaming_state_for_new_turn`（L477）との違い: 後者は `last_message_id` と `task_id_map` も消すが、本ヘルパーは post-turn のため両者を**残す**。`streaming_message_id` は `run_turn_complete_transition_locked` 内で既に `take()` 済みのため触らない。

### 再シードヘルパー（責務）

```rust
/// post-turn イベント処理の起点で、空の streaming_parts に
/// 「last_message_id に対応する永続メッセージの parts」を再投入する。
/// 戻り値は base を投入できたか（メッセージが見つかったか）。
fn reseed_post_turn_base_from_store<R: tauri::Runtime>(
    session_store: &SessionStore,
    app: &tauri::AppHandle<R>,
    proc: &mut AgentProcess,
    chat_session_id: &str,
    message_id: &str,
) -> bool {
    // resolve_data_dir → get_session → messages.find(id).parts を
    // proc.streaming_parts へ move。読み出し失敗時は false（後述のフォールバック）。
}
```

呼び出し条件: post-turn 分岐で `post_turn == true && proc.streaming_parts.is_empty()` のとき（= 完了直後または直前 post-turn 処理でクリアされた状態）。`accumulate_sdk_message` の**前**に呼ぶ。これにより `prev_len = streaming_parts.len()` が base 長となり、delta は新規 post-turn 分のみになる。

## データモデルまたは型

`AgentProcess` の**フィールド追加・削除・型変更はしない**（R4）。`streaming_parts: Vec<MessagePart>` の保持ライフタイムだけを変える。`MessagePart` / `TurnCompleteTransition` / プロトコル DTO も不変。

post-turn の base は「永続化済み `ChatMessage.parts: Option<Vec<MessagePart>>`」を単一の真実源とする。メモリ上に base 専用フィールドを増設しない（増設は実質的に常駐を温存するため不採用、§リスク参照）。

## 処理フロー

### ターン完了（両経路共通ヘルパー）

```
run_turn_complete_transition_locked:
  was_streaming = flush_streaming_before_transition(...)   // 既存: 最終 delta を emit
  state / turn_phase 更新                                   // 既存
  turn_token_usage = last_result_token_usage.take()        // 既存
  final_parts = consolidate_parts_from_slice(streaming_parts)  // 既存スナップショット
  final_msg_id = streaming_message_id.take()               // 既存
  if final_msg_id.is_some(): last_message_id = final_msg_id // 既存（post-turn 用）
  if exit_code != 0 || pending_stream_part_count == 0:
      release_completed_turn_streaming_buffer(proc)        // ★追加: 異常終了または pending なしなら即時解放
  else:
      streaming_parts を保持                            // 正常完了かつ pending>0。post-turn 処理で解放
  return TurnCompleteTransition { ... }                    // 既存（final_parts 等は move 済み）
```

完了後の最終 persist（`persist_streaming_parts(mid, &final_parts, Some(completed_at))`）は呼び出し側で `final_parts`（ヘルパーが返したスナップショット）を使うため、即時解放分岐で `streaming_parts` をクリアしても emit/persist 内容は不変（R3）。`final_parts` は `streaming_parts` のクリア前に確定済み。正常完了かつ `pending_stream_part_count > 0` の場合は、pending 分の post-turn payload 確定まで parts を残し、post-turn 分岐の `release_completed_turn_streaming_buffer(proc)` で解放する。したがって R1 の保証時点は「常にターン完了直後」ではなく、「即時解放条件を満たす完了直後、または post-turn 処理完了後から次ターンまでの idle 期間」である。

### post-turn イベント（両経路の `_ =>` 分岐）

```
in_streaming = state==Streaming && streaming_message_id.is_some()
post_turn   = !in_streaming && last_message_id.is_some()
if !in_streaming && !post_turn: 無視（既存）
else:
  if post_turn && streaming_parts.is_empty():
      reseed_post_turn_base_from_store(store, app, proc, csid, last_message_id)  // ★base 再構築
  prev_len = streaming_parts.len()                          // base 長
  (acc, updated) = accumulate_sdk_message(msg, streaming_parts, task_id_map)  // 既存
  delta = streaming_parts[prev_len..] (+updated)            // 既存: 新規分のみ
  mid = in_streaming ? streaming_message_id : last_message_id  // 既存
  enqueue_pending_delta(delta); flush（emit 累積全体）        // 既存: emit は base+delta
  persist_parts = consolidate(streaming_parts)              // 既存: 累積全体
  if post_turn: release_completed_turn_streaming_buffer(proc)  // ★payload 確定後に再解放
→ ロック外で persist_streaming_parts(mid, &persist_parts, None)  // 既存（内容不変）
```

ポイント:
- emit（`flush_streaming` → `emit_streaming_parts`）はロック内で `streaming_parts`（= base+delta）を読むため、再クリアは emit の**後**かつ persist payload 確定の**後**に行う。
- `persist_parts` はロック内で確定（`consolidate(streaming_parts)`）してから move するため、再クリアの影響を受けない。persist 本体はロック外・同期呼び出しで、次イベント処理より前に完了する。
- 次の post-turn イベントは `streaming_parts` 空 → 直前 delta を含む最新永続メッセージから再シード。複数連続でも累積保持なし・内容一致（R2 の複数 post-turn シナリオ）。

### consolidate 等価性（R2/R3 の不変保証）

旧挙動の persist/emit payload は `consolidate(raw_full ++ delta)`（`raw_full` は元ターンの未マージ生 parts 列）。新挙動は `consolidate(consolidate(raw_full) ++ delta)`。`consolidate_parts_from_slice` は隣接同型（text/thinking、同 `parent_tool_use_id`）の連続のみマージし、内容・境界型を保存する冪等変換のため、

```
consolidate(consolidate(A) ++ B) == consolidate(A ++ B)
```

が成り立つ（A 内部の境界はマージ済みで内容不変、A 末尾と B 先頭の跨ぎマージは両式とも末尾型／先頭型のみに依存）。よって base を「生 parts 列」から「consolidated 列」へ置換しても emit/persist payload は完全一致する。これが「メモリは減るが外部振る舞いは不変」の根拠。

### 履歴復元（R2 / 不変）

`get_session_internal_with_data_dir`（L3437-)は `proc.state == BridgeState::Streaming` のときだけ `streaming_parts` を参照し、それ以外は `Vec::new()` を返して永続 `session.messages` を使う。ターン完了後は `state == Ready`（または `Crashed`）のため、`streaming_parts` のクリアは get_session（履歴復元）に影響しない。復元内容は従来どおり永続ストア由来で不変。

## エラー処理

- **base 再シードの読み出し失敗**（`resolve_data_dir` / `get_session` エラー、メッセージ未発見）: `reseed_post_turn_base_from_store` は `false` を返し、`streaming_parts` は空のまま継続する。この場合 post-turn delta のみで emit/persist され、`persist_streaming_parts` が `msg.parts` を delta のみで上書きする risk（base 欠落）が生じる。**緩和**: 読み出し失敗時は `log::warn!` を出し、当該 post-turn イベントの **emit/persist をスキップ**（accumulate せず無視）して既存メッセージを破壊しない。表示の即時性より「確定内容を欠落・上書きしない」（R2）を優先する。失敗は I/O 異常時のみで通常経路では発生しない。
- **完了時に final persist がスキップされるケース**: 最終 persist は `was_streaming && !final_parts.is_empty()` のときのみ。`post_turn` 成立条件は `last_message_id.is_some()`（= 完了時に `streaming_message_id` が存在）であり、その場合 `state == Streaming`（`was_streaming == true`）かつストリーミング中の周期 persist（L3128 / L5978）でメッセージは既に永続化済み。よって base 再シードは少なくともストリーミング末尾相当の parts を得られる。`final_parts` が空（内容なし）の稀ケースでは base 空のまま post-turn parts が当該メッセージ本体になる（旧挙動と同等）。
- **既存のエラー分岐**（`"error"` メッセージ、permission、token usage 等）は変更しない。`"error"` 分岐は `state == Streaming` 時のみ `streaming_parts` を触るため、完了後クリアと干渉しない。
- ロック保持区間は既存と同じ（accumulate〜flush〜persist_parts 確定をロック内、persist 本体をロック外）。再シード読み出しはロック内で同期 I/O を行うが、これは既存の post-turn persist と同じ I/O クラスであり、保持時間の増分は session 1 件の読み出し分に限定される。

## テスト方針

`#[cfg(test)] mod tests`（同ファイル内）に追加。既存テスト（`reset_streaming_state_for_new_turn_clears_all_coalescing_state` L8110、turn-complete transition L8774 付近、`consolidate_parts_from_slice` 系 L12124-）は不変のまま green を維持する（R5）。

新規テスト:

1. **正常系・即時解放（AC1 / R1）**: `streaming_parts` に複数 parts を積み、`pending_stream_part_count == 0` の proc に対し `run_turn_complete_transition_locked` を実行 → 戻り値 `final_parts` が従来同等の consolidated 内容であり、かつ `proc.streaming_parts.is_empty()` を assert。`last_message_id` が保持されていることも assert。
2. **post-turn 再シード → 内容一致（AC2 / R2）**: 完了で解放済み（`streaming_parts` 空、`last_message_id = mid`）の状態を作り、永続ストアに base parts を持つメッセージを用意。post-turn delta を accumulate する経路を駆動し、emit/persist に渡る consolidated payload が「base ++ delta の consolidated」と一致することを assert（旧挙動＝ base を保持し続けた場合の payload と同値）。
3. **pending あり完了 → post-turn 解放（R1 / R2 複数シナリオ）**: 正常完了かつ `pending_stream_part_count > 0` では `run_turn_complete_transition_locked` 直後に parts が保持されることを assert。その後 2 件以上の post-turn delta を順に処理し、各処理後に `streaming_parts` が再び空であること、最終的に永続化されるメッセージ内容が全 delta を欠落・重複なく含むことを assert。
4. **解放等価性（R3）**: 同一の生 parts 列に対し「旧: 保持し続けて consolidate」「新: 解放→再シード→consolidate」の emit/persist payload が一致することを、`consolidate(consolidate(A)++B) == consolidate(A++B)` のプロパティとして検証するユニットテスト。
5. **再シード失敗フォールバック（エッジ）**: get_session が None／エラーのスタブで post-turn を駆動し、既存メッセージを破壊せず（persist スキップ）warn 経路に入ることを確認。

`SessionStore` 依存テストは既存テストのスタブ／一時ディレクトリ方式（`git2::Repository::init` 等の既存パターン）に合わせる。Tauri `AppHandle` を要する経路は既存テストのモック方針（emit closure 注入で同一コードパスを駆動）に倣う。

実機検証（AC5・任意）: 修正前 HEAD と修正後ビルドで、長時間アイドルを含む会話進行を実行し、完了ターン分相当の常駐 RSS が解放されることを補助的に確認する。必須合格条件は R1〜R5。

品質チェック: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、および `pnpm lint` / `pnpm test`（フロント無変更だが CI と同条件で green を確認）。

## リスクと代替案

### リスク

- **R-1: post-turn 毎の追加 I/O**。base 再シードで session 読み出しが post-turn イベント毎に 1 回増える。post-turn persist が既に `get_session`+`save_session` を都度行うため増分は限定的だが、post-turn イベントが高頻度な session では I/O が増える。緩和: 再シードは `streaming_parts` 空時のみ（in-streaming 中は発生しない）、かつ post-turn は背景タスク完了等で本質的に低頻度。
- **R-2: 再シード元の鮮度**。base は永続メッセージに依存するため、完了時 final persist と周期 persist の網羅性に依存する（§エラー処理で整理済み）。final persist スキップの稀ケースでは末尾 1 persist 分の遅延が理論上あり得るが、`post_turn` 成立時は周期 persist 済みでメッセージは存在する。
- **R-3: 逐次処理前提**。post-turn の「クリア→次イベントで再シード」は、同一 session のメッセージが単一 reader タスクで逐次処理されること（`persist_streaming_parts` が同期関数で当該イベント内に完了すること）に依存する。現状コードは stdout reader / external handler ともに逐次のためこの前提は成立。並行化する将来変更時は要再検討（コメントで明示する）。

### 代替案（不採用）

- **A-1: ターン完了時に `streaming_parts = final_parts`（consolidated 化のみ）**。生 delta 列を consolidated 列へ縮約し I/O を増やさない。だが完了メッセージ内容はメモリに残り続け、R1 の「解放」と behavior.md の「バッファが空」を満たさない（部分削減に留まる）。**不採用**。
- **A-2: base 専用フィールド `post_turn_base: Vec<MessagePart>` を新設して保持**。`streaming_parts.is_empty()` は満たせるが、別フィールドに完了メッセージを常駐させるだけで実メモリは解放されず、テストの形式的充足になる（R1 の実解放意図に反する）。`AgentProcess` の型変更も伴う（R4 に逆行）。**不採用**。
- **A-3: emit/persist を delta 追記方式へ変更**（フロントが append、persist が既存 parts へ追記）。base をメモリに持たずに済むが、`agent-streaming-updated` / `AgentStreamSync` の「parts 全体置換」契約とフロント実装を変える必要があり、外部観測振る舞いの変更（R2/R3 違反）と影響範囲拡大（R4 違反）になる。**不採用**。採用方針は emit/persist の契約を一切変えず、in-memory の base を一時再構築することで payload を同値に保つ。

## 仮定

- 「外部から観測可能な振る舞い」= フロント emit イベント内容、SessionStore 永続メッセージ内容、履歴復元メッセージ内容、workflow turn-complete 通知内容（requirements / behavior と一致）。内部バッファ構造・複製回数・常駐量は含まない。
- 本 Issue の主対象は「ターン完了後アイドル session の `streaming_parts` 常駐」。close 経路はプロセスが `AgentProcessMap` から除去され解放されるため新規対象としない。
- post-turn は同一 session のイベントが単一 reader で逐次処理され、`persist_streaming_parts`（同期）が当該イベント処理内に完了する（R-3）。
- `post_turn`（`last_message_id.is_some()`）成立時、対象メッセージは完了時 final persist もしくはストリーミング周期 persist により永続化済みで、base 再シードで取得可能。
- Codex backend は `handle_external_bridge_message` 経由、Claude/legacy は stdout reader 経由で同一 post-turn ロジックを共有し、両経路へ対称適用する。
- `consolidate_parts_from_slice` は冪等かつ境界保存で、`consolidate(consolidate(A)++B) == consolidate(A++B)` が成立する（R3 の payload 一致根拠）。

## Open Questions

なし。
