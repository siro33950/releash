# Requirements

## Type

不具合修正（Agent チャット安定化 / milestone 84・Phase 0・依存なし）

## Goal

stalled turn（agent が長時間無反応）中にユーザーがメッセージを送信したとき、入力テキスト・添付画像・mention が失われないことを保証する。

送信操作の結果を「turn 開始 or queue 追加」の 2 つに収束させ（ideal lifecycle I6）、steer 非対応 backend への実行中送信・stall 判定中の送信のいずれでも `active-turn steering is not available` の生エラーがユーザーに露出せず、入力欄と添付が保全される状態を成功とする（OB-2 の解消）。

## Background

`specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` の **OB-2**（severity: high, 種別: dropped）で報告された不具合。

現状、`send_message` は stall 観測中（デフォルト 180 秒無出力、tool 実行中は 1800 秒へ延長）の turn に対し、backend が steering 非対応なら queue せずエラーを返す（`runtime/usecase.rs:292-297`）。Claude / Codex とも `capabilities().steering=false` で、どちらの `SessionRuntime` も `steer` をオーバーライドしていない（`gateway.rs:160-165` の既定が `Unavailable`）ため、steer 分岐は本番で到達不能であり、stalled turn への送信は常にこのエラーになる。この時点では `add_human_message_internal` 前のため、メッセージは永続化されず完全に失われる。

frontend 側にも取りこぼしが重なっている:

- `MessageInput.tsx:440-456` は `onSend` を await せず、送信直後に入力欄・添付画像・mention を即クリアする。
- `useAgentChat.ts:913-919` の catch は `SET_ERROR` でバナー表示するのみでエラーを swallow するため、送信失敗を入力側へ伝播できず、入力を復元する経路が存在しない。
- stall 中は `ChatSessionView.tsx` が「No agent output…」バナーを表示しつつ `MessageInput` を無効化しないため、まさにユーザーの追加指示を誘発した上で、本文と画像を復元不能に破棄する。

これは通常の実行中なら queue に積まれる操作が、stall 中だけ入力ごと捨てられるという非対称であり、agent が無反応で追加指示を送りたい状況でこそ発火する。

ideal lifecycle **I6（ユーザー入力の無損失）** は「送信操作は成功（turn 開始 or queue 追加）以外の結果を持たない。steer 非対応・stall 中・起動失敗のいずれでも入力テキスト・添付画像は失われない」と定義し、stalled 判定中も同一経路（queue）に載せることを要点とする。ideal presentation **P5（入力の保全）** は「送信失敗・queue 操作・stall のいずれでも入力欄の内容と添付は消えない」と定める。

なお、backend が stall 中の送信でエラーを返す現挙動は、`runtime/usecase.rs:7568-7592` のテスト（`test_stale_watchdog_無進捗turnをstall_signalに留めruntimeを閉じない`）で「stalled retry/continue must not be silently queued」という意図的仕様として固定されている（issues-1301 D16/F-2）。本 ISSUE では、ideal lifecycle I6 を優先し、この既存の意図的仕様を反転させる（stall 中の送信も queue へフォールバックする）ことを合意済みとする。当該テストは新仕様に合わせて更新／置換する。

## Users / Actors

- Releash デスクトップアプリのエンドユーザー（Agent チャットで stalled 中に追加指示・催促を送るユーザー）
- backend runtime（Claude / Codex agent session）
- 本コードベースを保守する開発者

## Scope

- backend（`runtime/usecase.rs` の送信経路）: 実行中 turn への送信が steer 未対応（`steer` 既定 Err / steering 非対応 backend）となる場合に、エラーを返さず pending queue へ積む。stall 判定中の送信も同一経路に載せ、エラーにしない（I6）。
- frontend の入力保全（P5）:
  - 送信ハンドラが送信 API の完了を待ち、成功応答を得た場合にのみ入力欄・添付画像・mention をクリアする。
  - 送信 API が失敗した場合は入力欄の内容・添付・mention を保持する（現状の即時・無条件クリアを撤廃）。
  - stall 中の送信が queue チップとして表示され、失われないこと。
- backend エラー文言 `active-turn steering is not available` がユーザー UI に露出しないこと。
- テスト: stalled 状態を模擬し、送信 → queue 追加 → turn 終了後に queue が実行される流れを固定する（統合／backend テスト）。既存の相反テスト（issues-1301 D16/F-2、`runtime/usecase.rs:7568-7592`）は、新仕様（stall 中も queue へフォールバック）に合わせて更新／置換する。

## Non-goals

- pending queue の永続化（OB-3: session close / backend 切替 / 再起動での queue 消滅）。本 ISSUE では扱わない。
- cancel 済みメッセージの transcript 復活（OB-4）、interrupt 後の無条件 drain（OB-5）、queue 起動失敗後の自動リトライ欠如（OB-6）、画像のみ送信時の wire 非対称（OB-7）。いずれも別 ISSUE。
- turn/steer の将来対応そのもの（backend が実際に steering を実装すること）。本 ISSUE は「steer 非対応時のフォールバック」に限る。
- WebSocket 経由で agent message を送信する inbound protocol / controller surface の追加。将来この surface を追加する場合は、同じ backend usecase と `SendMessageResponse` を再利用する。
- stall 判定ロジック（無出力タイムアウト値・tool 実行中の延長）の変更。
- Agent チャット以外の送信経路の変更。

## Requirements

- **R1（backend フォールバック）**: 実行中 turn への送信が steer 未対応となる場合、送信をエラーにせず pending queue へ積む。steering 非対応 backend（Claude / Codex）・stall 判定中のいずれでも同一の queue 経路に載せ、`active-turn steering is not available` エラーを返さない。
- **R2（メッセージ永続化）**: queue へ積む際、human message を永続化し、queue entry と紐づける（既存の実行中送信と同じ扱い）。入力テキスト・添付画像・mention・editor_context を欠落なく保全する。
- **R3（frontend クリアの成功条件化）**: 送信ハンドラは送信 API の完了を待ち、成功応答を得たときにのみ送信対象の入力欄・添付画像・pasted text・mention をクリアする。送信 API が失敗した場合と、送信待機中に追加された入力状態は保持する。同一の送信操作を完了前に重複送信しない。
- **R4（失敗の伝播）**: 送信 API の失敗が送信ハンドラ（入力保全の責務を持つ層）へ伝播し、入力を復元／保持できること。エラーを swallow して入力復元経路を失わないこと。
- **R5（queue チップ表示）**: stall 中の送信が queue チップとして UI に表示され、ユーザーから見て失われていないことが分かること。
- **R6（生エラー非露出）**: `active-turn steering is not available` を含む backend 内部エラー文言が、ユーザー向け UI にそのまま露出しないこと。
- **R7（テスト固定）**: stalled 状態での送信 → queue 追加 → turn 終了後の queue 実行、および送信失敗時の入力保全を、テストで固定する。
- **Rust-first**: 送信の queue 判定・queue 投入・永続化ロジックは Rust（usecase）に置く。frontend は成功/失敗に応じた入力クリア／保持という UI 制御のみを担う。

## Constraints

- 全アプリケーションロジックは Rust に置く（`.claude/rules/rust-first-logic.md`）。queue 判定・フォールバックの意思決定を frontend に実装しない。frontend の変更は「送信結果に応じた入力欄クリア／保持」という UI 制御に限る。
- pending queue の所有者は backend（`RuntimeSessionState.pending_queue`）である。frontend は queue 状態の mirror に留める。
- full-retention / full-recompute 経路を新設しない。queue 投入は既存の QueuedTurnInput 経路を再利用する。
- 送信結果は backend-owned な `SendMessageResponse` とし、現在の Tauri command 以外の将来 surface からも usecase を再利用できる境界を維持すること。
- 既存 CI 品質チェック（`pnpm lint` / `pnpm test` / `pnpm build`、`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、`pnpm test:integration`）が通過すること。

## Success Criteria（受け入れ基準の概要）

- stall 中の送信が queue チップとして表示され、メッセージ（本文・添付・mention）が失われない。
- 送信失敗時も入力欄の内容と添付が残る。
- 「steering is not available」エラーがユーザーに露出しない。
- 上記を固定するテストが存在し、通過する。
- CI 品質チェックが通過する。

## Assumptions

- spec-id は `issues-1403` とする（ブランチ `feat/issues/1403` および既存 `docs/specs/issues-<番号>/` 命名規約に整合）。
- 本 ISSUE は milestone 84「Agent チャット安定化」Phase 0・依存なしで即着手可能であり、正本は `specs/milestone-84-agent-chat-stabilization/`（audit OB-2 / lifecycle I6 / presentation P5）である。
- 「queue へ積む」対象は、backend が steering 非対応の実行中 turn（stall 判定中を含む）への送信とする。将来 backend が実 steer を実装した場合も、本 ISSUE で steer 経路を優先するのは stall 観測中の active turn に限り、通常の streaming turn は既存どおり queue へ積む。
- frontend の送信ハンドラ改修は、`MessageInput.tsx` の即時クリアと `useAgentChat.ts` の catch swallow の 2 点が対象で、入力保全に必要な最小範囲に限る（クリアを成功応答後に移す／失敗を伝播させる）。
- queue 投入時の human message 永続化・queue 紐付けは、既存の実行中送信（`runtime/usecase.rs` の queue 分岐）と同一の仕組みを再利用する（新規の永続化スキーマは追加しない）。

## Decisions（解消済み）

- **既存の意図的仕様（issues-1301 D16/F-2）の上書き**: ideal lifecycle I6 を優先し、stall 中の送信も queue へフォールバックする方針（R1）を採用する。これにより issues-1301 D16/F-2（「stalled retry/continue must not be silently queued」、`runtime/usecase.rs:7568-7592`）を反転させ、当該テストは新仕様に合わせて更新／置換する。audit の代替案（backend はエラー維持・frontend のみで保全）は採らない。

## Open Questions

なし。
