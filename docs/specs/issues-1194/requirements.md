# Requirements

## Type

メモリ削減（不具合修正）。ライフタイム／解放の設計判断を伴う蓄積経路の是正。#1191 の広域調査で特定した候補 C1 を分離した ISSUE（#1194）。

## 背景と目的

### 背景

- #1191（Codex backend 会話のメモリ枯渇）の広域調査で、無制限に増大しうる構造的経路が複数特定された。#1191 本体は振る舞い不変・低リスクな純粋削減（A群: C2/C3/C4/C7/C8/C9）に絞り、ライフタイム／解放の設計判断を要する候補（B群: C1/C6/C10）を別 ISSUE へ分離した。本 ISSUE はそのうち **C1** を扱う。
- 対象経路: `AgentProcess.streaming_parts: Vec<MessagePart>`（`src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`）が**ターン完了時に解放されない**。
- 実コードで確認した現状（要約。詳細は `design.md` で扱う）:
  - `streaming_parts` の `clear` は次ターン開始時の `reset_streaming_state_for_new_turn`（L477-484）のみ。
  - ターン完了処理 `run_turn_complete_transition_locked`（L1237 付近）は `consolidate_parts_from_slice(&proc.streaming_parts)` で確定 parts をスナップショットするだけで、`streaming_parts` 自体は `clear` しない。
  - 通常のターン完了では `AgentProcessMap` から process は除去されず（除去は明示 close 時）、**次ターンが来ない session では完了ターン分の全 parts が常駐し続ける**。
  - post-turn のバックグラウンドイベント（L3054-3137 付近の `_` 分岐、`post_turn = !in_streaming && proc.last_message_id.is_some()`）は、同じ `streaming_parts` へ追記し、`consolidate_parts_from_slice(&proc.streaming_parts)` で**メッセージ全体**を再確定して emit／persist する。
- 結果として、会話全体（アイドル時も直前ターン分が常駐）＋ post-turn 追記分 ＋ 複数 session で積み上がる**常駐型のメモリ増幅**が生じる。

### 目的

Codex backend との会話で、ターン完了後にアイドル状態の session が抱える `streaming_parts` 常駐メモリを解放し、会話・session 数の増加に伴って完了ターン分のストリーミング parts が際限なく常駐し続けない状態にする。これを、ストリーミング表示・イベント永続化・履歴復元の**外部から観測可能な振る舞いを不変に保ったまま**達成する。

## スコープ

- ターン完了時に、確定済み `streaming_parts`（完了ターン分の累積 parts）を常駐させ続けない解放設計を導入する。
- 解放後に到着する post-turn イベント（バックグラウンドタスクのツール完了・status 等）の表示・永続化を、解放前と同じ外部振る舞いで継続できるようにする（例: 完了時に確定 parts を解放し、以降の受信を別の小バッファ＋既存メッセージへの追記で扱う、等の解放設計）。具体的手段は `design.md` で確定する。
- 上記変更の検証手段（既存振る舞いの非退行確認と、完了ターン分が常駐しないことの確認）を整備する。

## 非スコープ

- #1191 の A群（C2/C3/C4/C7/C8/C9）純粋削減そのもの（別 ISSUE で実装済み／対応中）。
- C6（フロント全メッセージ保持の退避／仮想化）→ #1195。
- C10（terminal 後の workflow execution 解放）→ #1196。
- ストリーミング表示・イベント永続化・履歴復元の機能仕様そのものの変更・再設計。
- Codex 以外の agent backend 固有のメモリ問題の調査・修正（共通経路の是正で副次的に解消されるのは可）。
- メモリ使用量の新規モニタリング機能・常設プロファイラの製品化。
- `pending_approval_methods` クリア漏れ等、本 ISSUE の主目的ではない既存バグの修正。

## 要求事項

- R1: ターン完了後、次ターンが来ない（アイドル）session において、完了ターン分の確定 `streaming_parts` が `AgentProcess` 内に常駐し続けないこと。会話・session 数が増えても、完了済みターン分のストリーミング parts が解放されず累積し続ける常駐経路が解消されること。
- R2: ターン完了後に到着する post-turn イベントの、フロントへの表示・永続化（メッセージへの反映）の外部振る舞いが、本変更の前後で変化しないこと。解放によって post-turn イベントが既存メッセージの確定内容を欠落・上書き・重複させないこと。
- R3: ターン完了時にフロントへ送出される確定メッセージ（`final_parts` 経由の emit／persist、workflow turn-complete 通知）の内容が、本変更の前後で変化しないこと。
- R4: 本変更は C1（`streaming_parts` のターン完了時解放）に必要な範囲に限定し、クリーンアーキテクチャの構造や無関係な経路を不必要に変更しないこと。
- R5: 既存テスト（`cargo test` / `pnpm test`）および lint（`cargo clippy -- -D warnings` / `pnpm lint`）が green であること。新規ロジックには正常系・post-turn 追記を含むエッジ系のテストを追加すること。

## 受け入れ基準の概要

- AC1: ターン完了後にアイドルとなった session で、完了ターン分の `streaming_parts` が解放され常駐しないことを、テストまたはコード上の不変条件として確認できる（R1）。
- AC2: 解放後に post-turn イベントが到着するシナリオで、表示・永続化される最終メッセージ内容が解放前と一致することをテストで確認できる（R2）。
- AC3: ターン完了時の確定メッセージ emit／persist／workflow 通知の内容が本変更前後で不変であることを、既存テストおよび追加テストで確認できる（R3）。
- AC4: 既存テスト・lint が green（R5）。
- AC5（任意・確認用）: 修正前 HEAD と修正後ビルドの比較で、長時間アイドルを含む会話進行時に完了ターン分の常駐メモリが解放されることを実測で確認できる。実測はクラッシュ完全解消の保証ではなく、C1 解放の有効性確認に用いる。

## 仮定

- 本 ISSUE の主たる是正対象は「ターン完了後にアイドル session が抱える `streaming_parts` 常駐」であり、close 時は既に process が `AgentProcessMap` から除去される（`reset_streaming_state_for_new_turn` 相当の解放を含む）ため、close 経路は新規の主対象としない。
- 受け入れの中核は「常駐経路の解消（R1）＋ 外部振る舞いの不変（R2/R3）＋ 既存テスト・lint green」とする。実メモリ計測（AC5）は補助的な確認であり、本 ISSUE 単体の必須合格条件としては R1〜R5 を基準にする（#1191 が確立した「診断は実測、是正は振る舞い不変」の運用に整合）。
- post-turn イベントの解放設計（確定 parts 解放後に別バッファ＋既存メッセージ追記で扱う等）は要求を満たす一手段であり、最終的な手段選定は `design.md` で確定する。本書は手段を一つに固定しない。
- 「外部から観測可能な振る舞い」とは、フロントへの emit イベント内容、SessionStore への永続化メッセージ内容、履歴復元時に再現されるメッセージ内容を指す。内部のバッファ構造・複製回数・メモリ常駐量はこれに含めない。

## Open Questions

なし。
