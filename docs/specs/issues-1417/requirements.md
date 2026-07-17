# Requirements

関連: #1417 ([Agentチャット安定化] X1: 画像のみ送信時の空 text block 修正)

## Type

不具合修正（backend / Rust）。Claude backend の turn 入力（wire）組立てを Codex と対称化し、画像のみ（本文なし）送信時に空 text block を送らないようにする。外部から観測可能な UI の追加はなく、既存の「画像のみ送信」操作の成否を backend 間で一致させる。

## 背景と目的

milestone 84「Agentチャット安定化」の Phase 0（依存なし・即着手可）。監査ドキュメント `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` の **OB-7**（重大度 low / 種別: divergent）を解消する。

現状、スクリーンショットだけを添付してテキストなしで送信すると、backend によって成否が分かれる非対称がある。

- **Claude**: `user_message`（`claude/wire.rs:140-167`）が `prompt` の空・非空にかかわらず、常に `{"type":"text","text":""}` を content 先頭へ push する。Anthropic Messages API は空 text block を `invalid_request_error` で拒否する既知の挙動があり、画像のみ送信時に turn が API エラーで失敗し得る（claude CLI がこれをサニタイズするかはリポジトリ外の挙動のため未検証だが、非対称そのものが問題）。
- **Codex**: `codex_user_input`（`codex/session.rs:574-586`）が `!prompt.is_empty() || images.is_empty()` の条件で text item を積み、画像のみ送信時は空 text を送らないよう明示的にガードしている。

frontend は画像のみ（本文空）の送信を許可し（`MessageInput.tsx:437-439`）、backend の usecase / adaptor 層にも user prompt の空バリデーションはなく、`claude/session.rs:212` で `input.prompt` が無加工で `user_message` に渡る。結果、同じ「画像のみ送信」操作が Claude backend でだけ失敗し得る。

本変更の目的は、この wire 生成の非対称を解消し、Claude backend でも画像のみ送信が Codex と同じく成功するようにすることである。

### 修正方針（Issue・監査の指定）

`claude/wire.rs` の `user_message` に Codex と同じ条件を入れる。すなわち **`prompt` が非空、または `images` が空のときだけ** text block を push する（画像ありかつ prompt 空のときは text block を省略する）。これにより両 backend の wire 生成条件が対称になる。

## スコープ

1. `claude/wire.rs` の `user_message` を修正し、`prompt` が空かつ `images` が非空のときは content 先頭の text block を生成しない（Codex `codex_user_input` と対称の条件 `!prompt.is_empty() || images.is_empty()`）。
2. 次の 2 ケースの挙動をテストで固定する。
   - 空 prompt ＋ 画像あり: text block を含まず image block のみの content になる。
   - 空 prompt ＋ 画像なし: 既存挙動に従う（Codex と対称に、空 text block を 1 つ含む content になる。送信可否の判断は frontend 側の既存挙動に委ね、本 backend 修正では変更しない）。
3. 修正ロジックは Rust/backend 側（`wire.rs`）に置く。frontend は変更しない。

## 非スコープ

- frontend（`MessageInput.tsx` / `useAgentChat.ts`）の送信バリデーションや UI の変更。画像のみ送信の許可可否は現状の frontend 挙動を維持する。
- usecase / adaptor 層への user prompt 空バリデーションの新規追加。
- Codex backend 側の挙動変更（既にガード済み）。
- 空 text block 以外の Anthropic API エラー全般や、claude CLI のサニタイズ挙動そのものの検証・対処。
- OB-7 以外の監査項目（OB-8 以降・RT 系など）の対応。

## 要求事項

1. Claude backend の turn 入力組立てにおいて、prompt が空かつ画像ありのとき、user message content に空 text block を含めない。
2. prompt が非空のとき、または画像がないときは、従来どおり text block（内容は prompt 文字列、空文字を含む）を content 先頭に含める。
3. 画像がある場合の image block の生成・並び順（text の後に image を並べる）は従来どおり維持する。
4. 上記条件は Codex の `codex_user_input` と論理的に対称であること。
5. 修正対象・追加テストは backend（Rust）に閉じ、frontend を変更しないこと。

## 受け入れ基準の概要

- スクリーンショットのみ（本文なし）の送信で、Claude backend が生成する user message content に空 text block が含まれない（image block のみになる）。
- 空 prompt ＋ 画像あり、空 prompt ＋ 画像なしの 2 ケースが unit test（`wire.rs` の `#[cfg(test)]`）で固定され、Codex 側の条件と一致する。
- 両 backend で「画像のみ送信」の成否が一致する（Claude でも API エラー要因の空 text block が送られない）。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- 「画像のみ送信」時に問題を起こす空 text block の生成箇所は `claude/wire.rs` の `user_message` のみであり、`claude/session.rs:212` はこれを無加工で呼ぶだけである（監査の記述と一致）。
- 空 prompt ＋ 画像なしのケースで空 text block を残す挙動は許容される（Codex も同条件で空 text を送る対称挙動であり、かつ frontend が本文・画像とも空の送信を通常許可しないため実運用上到達しにくい）。この対称性の維持を優先し、backend 側での送信拒否は本 Issue に含めない。
- 監査が指摘する「Anthropic API が空 text block を拒否し得る」挙動は、非対称解消の根拠として扱い、CLI のサニタイズ有無の外部検証は本変更の完了条件に含めない。

## Open Questions

なし
