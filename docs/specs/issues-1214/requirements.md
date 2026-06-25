# Requirements

## Type

性能・メモリ効率改善（通信プロトコルの変更を伴うリファクタリング）。

対象 Issue: #1214

正本ドキュメント: `docs/releash-performance-architecture-audit.md`（M2 / 項目 5・6）
マイルストーン: 性能・メモリ効率改善（Workbench State / Read Model）（#80）

## 背景と目的

### 背景

`docs/releash-performance-architecture-audit.md` の M2（項目 5・6）が指摘するとおり、現在の Agent streaming は **累積 parts のスナップショットを毎フレーム送る** モデルのため、応答が長くなるほど 1 frame あたりの仕事量と payload が線形に増大する。

実コードで確認した現状（要約。詳細は `design.md` で扱う）:

- Rust 側は `STREAMING_EMIT_INTERVAL_MS = 33`（`src-tauri/src/infrastructure/agent_session/runtime/bridge_common/stream_emit.rs`）で約 33ms ごとに timer tick し、`AgentProcess.streaming_parts` に追記された **累積 parts 全体** を `consolidate_parts_from_slice`（`bridge_common/shared.rs`）で統合し直してから emit する。
- emit は Tauri event `agent-streaming-updated` と、WebSocket 向けの `WsBroadcaster::send_stream_sync`（`src-tauri/src/ws_bridge.rs`）の双方で、`AgentStreamSync { session_id, message_id, parts }`（`src-tauri/src/protocol/agent.rs`）として **その時点の累積 parts 配列全体** を送る。
- frontend は `useAgentSdkListeners.ts` で `agent-streaming-updated` を受信し、`agentChatReducer.ts` の `SET_STREAMING_MESSAGE` で当該メッセージの `parts` を **受信した配列で丸ごと置換** する（delta ではなく累積スナップショット適用）。
- `seq` / `since_seq` / delta / streaming の resync は現状未実装。reconnect 時は `ws_bridge.rs` で stale な snapshot slot を clear する程度の部分対応のみ。
- 完了ターン分 parts のランタイム常駐については #1194 が turn 完了時の `streaming_parts` 解放を扱う。

この累積スナップショット方式は、#970（ストリーミング表示の即時性）で導入した 33ms coalescing と整合する一方で、応答長に比例して frame あたりの clone / consolidate コストと payload が増える構造的問題を抱える。

### 目的

通常配信を **累積 snapshot から `seq` 付き delta event へ移行** し、ストリーミング中の 1 frame あたり payload と処理量が応答全体長に比例して増えない状態にする。reconnect / resync 時だけ snapshot を送って `since_seq` から復元できるようにし、#970 の表示即時性を保ったまま、#1194 のランタイム解放と整合させる。あわせて `parts_to_legacy` を compatibility 出力に限定し、保存正典から外す。

## スコープ

- **通常配信の delta 化**: ストリーミング中の通常配信を、累積 parts スナップショットではなく `seq` 付き delta event にする。delta は順序づけ・欠落検知・重複排除が可能な単調増加 `seq` を持つ。
- **reconnect / resync 時の snapshot**: reconnect / resync 時に限り snapshot を送り、受信側が `since_seq` を起点に欠落分を復元できるようにする。通常配信中は snapshot を送らない。
- **#970 表示即時性の維持**: delta 化後も、frontend / remote が配信を受信した直後に UI へ反映される即時性（33ms coalescing 基準点）を保つ。
- **frontend と WS 配信の両方を delta 化**: frontend 向け Tauri event（現 `agent-streaming-updated`）と、残存する WebSocket 配信経路（`WsBroadcaster::send_stream_sync` / `AgentStreamSync`）の **両方** を、累積 snapshot 配信から delta + resync 配信へ移行する。delta を message store に適用し、重複・欠落・再接続を扱えるようにする（同一 `seq` の重複適用を冪等にし、欠落検知時は resync で復元）。delta 適用・順序づけ・重複排除のロジックは可能な限り Rust（read model / shared）側に置く（`.claude/rules/rust-first-logic.md`）。
  - モバイル向けフロント remote クライアントは削除済み（`CLAUDE.md`）のため、本 Issue は WS protocol / サーバー側の delta + resync 配信整備と、共有／Rust 側の delta 生成・適用ロジック整備までを対象とする。remote 専用クライアントの新規実装・E2E 検証は行わない（受信クライアント不在のまま、protocol・サーバーを将来のために delta 化する）。
- **`parts_to_legacy` の限定**: `parts_to_legacy` を compatibility 出力（互換目的の legacy 表現生成）に限定し、ストリーミングの保存正典・配信正典から外す。
- **#1194 解放との整合**: delta protocol が `streaming_parts` の累積常駐を再導入しないこと、および完了ターン分の復元（resync 用 snapshot 生成）を #1194 の turn 完了時解放と矛盾しない経路で成立させること。
- **検証手段の整備**: 通常 frame payload が応答全体長に比例しないこと、`since_seq` からの復元、表示即時性の非退行を確認する検証手段を用意する。

## 非スコープ

- ターン完了時の `streaming_parts` 解放そのものの実装（#1194 が担当）。本 Issue はその成果を前提とし、delta 化が解放と矛盾しないことを保証するに留める。
- session storage の summary index / message paging（`get_session_page` 等）の追加（#1213 が担当）。resync の復元源として paging / 永続化済み message を利用するのは可。
- frontend の閉じた session / 非表示 worktree の body 退避・仮想化・LRU（#1195 が担当）。
- `bridge_common.rs` の module 分割（#1217 が担当）。
- legacy `content` / `thinking` / `activities` の二重保持の全面廃止。`parts_to_legacy` を配信・保存正典から外すところまでが対象で、legacy 表現生成（互換出力）の完全撤去は別途扱う。
- Agent backend（Claude / Codex）の SDK delta 取得・resume 仕様そのものの変更。
- ストリーミング表示・履歴復元の UI 仕様・見た目の変更（性能・通信モデルの内部変更に限定）。
- メモリ／payload の新規モニタリング機能・常設プロファイラの製品化。

## 要求事項

- R1: ストリーミング中の通常配信が、累積 parts スナップショットではなく `seq` 付き delta event で行われること。1 frame（1 配信単位）の payload と処理量が、それまでに蓄積した応答全体長に比例して増加しないこと。
- R2: delta が単調増加する `seq` を持ち、受信側が順序づけ・欠落検知・重複排除を行えること。同一 `seq` の delta を重複受信しても、message store への適用結果が冪等であること。
- R3: reconnect / resync 時に限り snapshot が送られ、受信側が `since_seq` を起点に欠落した delta 範囲を復元できること。通常配信中は snapshot を送らないこと。
- R4: frontend 向け配信と WS 配信の両方が delta + resync で行われ、受信側が delta を message store に適用して重複・欠落・再接続を破綻なく扱えること。欠落検知時は resync により表示内容が正規の最終状態へ収束すること。WS については protocol / サーバー側が delta + resync を運べる状態にすることを範囲とし、remote 専用クライアントの実装・E2E 検証は対象外とする。
- R5: 本変更後も #970 の表示即時性が維持されること（配信受信直後に UI 反映され、行途中での停止やストリーミング完了時の一括表示が再発しないこと）。
- R6: `parts_to_legacy` が compatibility 出力に限定され、ストリーミングの配信正典・保存正典に使われないこと。delta / snapshot の正典が legacy 表現に依存しないこと。
- R7: delta protocol が `streaming_parts` の累積常駐を再導入しないこと。resync 用 snapshot の生成が #1194 の turn 完了時解放と矛盾しないこと。
- R8: ストリーミングで配信・適用される最終メッセージ内容（delta を順に適用した結果、および resync snapshot 適用後の状態）が、本変更前のスナップショット方式と外部から観測可能な範囲で一致すること。
- R9: ロジックは Rust（Tauri バックエンド）側に置く方針に従い、delta 生成・順序づけ・適用・重複排除・resync 復元のロジックを frontend に持ち込まないこと（表示用フォーマットは frontend 可）。
- R10: 既存テスト（`cargo test` / `pnpm test`）および lint（`cargo clippy -- -D warnings` / `pnpm lint`）が green であること。新規ロジックには delta 適用・欠落／重複・reconnect の正常系・エッジ系テストを追加すること。

## 受け入れ基準の概要

- AC1: 通常 frame の payload が応答全体長に比例して増えないことを、テストまたは計測で確認できる（R1）。delta 1 件の payload が、そのフレームで新たに生じた増分に概ね比例し、累積総量に比例しないこと。
- AC2: `seq` の欠落・重複・順序入れ替えを含むシナリオで、message store への適用結果が一意の最終状態へ収束し、重複適用が冪等であることをテストで確認できる（R2/R4）。
- AC3: reconnect 後に `since_seq` を起点とした resync snapshot で、切断中に欠落した delta 範囲を復元し、最終表示がスナップショット方式と一致することをテストで確認できる（R3/R8）。
- AC4: delta 化後も #970 の表示即時性（受信直後の UI 反映、行途中停止・一括表示の非再発）が保たれることを確認できる（R5）。
- AC5: `parts_to_legacy` が配信・保存の正典経路で使われていないこと（compatibility 出力限定）をコードまたはテストで確認できる（R6）。
- AC6: 既存テスト・lint が green（R10）。

## 仮定

- A1: 本 Issue は #1194（turn 完了時の `streaming_parts` 解放）の成果を前提とする。`streaming_parts` の解放実装そのものは #1194 が owner で、本 Issue は delta protocol がその解放と矛盾しないことの保証に範囲を限定する。
- A2: `seq` は `(session_id, message_id)` 単位で単調増加させ、delta はその message のストリーミング進行に沿って順序づけられるものと仮定する（最終粒度・採番起点は `design.md` で確定）。
- A3: resync 時に返す snapshot の復元源（runtime の bounded buffer か、永続化済み message / #1213 の paging 経由か）は `design.md` で確定する。本書は手段を一つに固定せず、「`since_seq` から欠落分を一意に復元できる」ことだけを要求する。
- A4: #970 で導入した 33ms coalescing は維持し、coalescing 後に「累積スナップショットを送る」のではなく「その間に生じた増分を `seq` 付き delta としてまとめて送る」形へ置き換える、という方向を基本線とする（具体的な delta 表現・event 形は `design.md`）。
- A5: 「外部から観測可能な振る舞い」とは、受信側 message store に最終的に適用されるメッセージ内容（parts）、表示の即時性、resync 後の収束状態を指す。内部の payload 形状・送信回数・clone 回数・メモリ常駐量はこれに含めない。
- A6: 通常配信の Tauri event 名・WS メッセージ型（現 `agent-streaming-updated` / `AgentStreamSync`）の新設・改廃や互換維持方針は `design.md` で確定する。本書は「通常 = delta、resync = snapshot」という配信の意味論のみを要求する。
- A7: モバイル向けフロント remote クライアントは削除済みのため、WS 配信の delta 化は protocol / サーバー側の整備として行い、受信クライアント不在のまま将来に備える。WS 経路の検証は Rust 側 protocol / 配信ロジックの単体・結合テストで担保し、remote クライアントを介した E2E は行わない。

## Open Questions

なし。
