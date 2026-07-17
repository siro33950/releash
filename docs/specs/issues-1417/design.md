# Design

関連: #1417 ([Agentチャット安定化] X1: 画像のみ送信時の空 text block 修正)

## 概要

Claude backend の turn 入力（user message wire）組立てを Codex backend と論理的に対称化し、
画像のみ（本文なし）送信時に空 text block を送らないようにする。

現状 `claude/wire.rs` の `user_message` は prompt の空・非空に関わらず常に
`{"type":"text","text":prompt}` を content 先頭へ push している。これを Codex の
`codex_user_input`（`!prompt.is_empty() || images.is_empty()`）と同じ条件に揃え、
**prompt が空かつ画像ありのときだけ text block を省略する**。それ以外の挙動は現状維持する。

外部から観測可能な UI 追加はなく、変更は backend（Rust）に閉じる。

## 変更対象

- `src-tauri/src/infrastructure/agent_session/claude/wire.rs`
  - `user_message`（現 `140-167`）の content 組立て条件を修正。
  - `#[cfg(test)] mod tests`（現 `169-192`）にケースを追加。

### 変更しないもの（非スコープ）

- `claude/session.rs:212`（`user_message` を無加工で呼ぶだけ。呼び出しは変更しない）。
- `codex/session.rs` の `codex_user_input`（既にガード済み。参照のみ）。
- frontend（`MessageInput.tsx` / `useAgentChat.ts`）の送信バリデーション・UI。
- usecase / adaptor 層の user prompt 空バリデーション（新規追加しない）。

## アーキテクチャと責務分割

- 修正ロジックは infrastructure 層の Claude wire 組立て（`claude/wire.rs`）に閉じる。
  これは Anthropic の stream-json wire フォーマットへの変換責務そのものであり、
  「空 text block を送らない」条件はこの変換の一部として妥当な配置である。
- 呼び出し元（`claude/session.rs`）は wire 生成関数へ prompt / images を渡すだけの
  責務に留め、条件分岐を持たせない（現状のまま）。
- Codex 側と論理的対称であることが要件のため、判定条件は Codex の
  `!prompt.is_empty() || images.is_empty()` と同一の真理値表を持つよう実装する。

## データモデルまたは型

`user_message` のシグネチャは現状維持する。

```rust
pub(crate) fn user_message(
    prompt: &str,
    images: impl IntoIterator<Item = (String, String)>,
) -> Value
```

- `prompt: &str` — 本文。空文字列 `""` を取り得る。
- `images: impl IntoIterator<Item = (String, String)>` — `(media_type, data)` の列。
  Codex の `codex_user_input` は `&[AttachmentPayload]` を受け取り `images.is_empty()` を
  直接判定できるが、Claude 側は `impl IntoIterator` のため長さを事前に知れない。

**仮定（実装方針）**: シグネチャは変えず、関数内で `images` を一度 `Vec` に collect して
`images.is_empty()` を判定する。これにより Codex と同じ条件式を使える。既存呼び出し
（`claude/session.rs:212` の `claude_images(input.images)`）はイテレータを一度だけ消費する
ため、collect による挙動変化はない。

content の要素型（JSON）は現状と同一:

- text block: `{"type":"text","text":<prompt>}`
- image block: `{"type":"image","source":{"type":"base64","media_type":<m>,"data":<d>}}`

## 処理フロー

1. `images` を `Vec<(String, String)>` に collect する。
2. `content: Vec<Value>` を空で初期化する。
3. `!prompt.is_empty() || images.is_empty()` が真のとき、text block を push する。
   （＝ prompt 空 かつ 画像あり のときだけ push しない）
4. collect 済み `images` を順に image block として push する（順序は従来どおり text の後）。
5. `{"type":TYPE_USER, "session_id":"", "parent_tool_use_id":null,
   "message":{"role":"user","content":content}}` を返す。

真理値表（Codex と一致）:

| prompt | images | text block |
|--------|--------|------------|
| 空     | あり   | 含めない   |
| 非空   | あり   | 含める     |
| 非空   | なし   | 含める     |
| 空     | なし   | 含める     |

## エラー処理

- 本変更でエラー経路は追加しない。`user_message` は `Value` を返す純粋関数であり、
  失敗経路を持たない。
- 監査が指摘する「Anthropic API が空 text block を `invalid_request_error` で拒否し得る」
  点は、非対称解消の根拠として扱う。API エラーそのもののハンドリングや CLI サニタイズ
  挙動の検証は本変更の完了条件に含めない（requirements の非スコープに準拠）。

## テスト方針

`claude/wire.rs` の `#[cfg(test)] mod tests` に unit test を追加し、4 ケースの content を固定する。

1. **空 prompt ＋ 画像あり**: `content` に text block を含まず、image block のみ。
   - `content[0]["type"] == "image"`、`content.as_array().len() == 画像数`。
2. **非空 prompt ＋ 画像あり**（既存テスト `test_claude_user_message画像をstream_json形式にする`
   を維持・活用）: `content[0]["text"] == prompt`、`content[1]["type"] == "image"`。
3. **非空 prompt ＋ 画像なし**: `content` は text block 1 つのみ、`content[0]["text"] == prompt`。
4. **空 prompt ＋ 画像なし（境界・現状維持）**: `content` は空文字 text block 1 つのみ、
   `content[0]["text"] == ""`。

- 外部プロセスは起動しない（純粋関数のテスト）。
- Codex `codex_user_input` との対称性は同じ真理値表を満たすことで担保する
  （Codex 側テストの有無に依存せず、Claude 側で 4 ケースを固定する）。
- 完了条件として `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を通す。

## リスクと代替案

- **リスク: collect による割当**。`images` を毎回 `Vec` へ collect するが、user message は
  turn ごとに 1 回・画像数も少数であり、性能影響は無視できる。
- **代替案 A: シグネチャを `&[(String, String)]` に変更**。呼び出し元で collect 済みを渡せば
  関数内 collect を避けられるが、`claude/session.rs` 側の変更が波及し、スコープが広がる。
  現状のイテレータ受け取りを維持し関数内 collect する方が変更が局所的で対称化の目的に十分。
- **代替案 B: 空 prompt ＋ 画像なしでも text block を省く**。Codex と非対称になり、要件
  （4 ケース目は空 text block を残す＝対称維持）に反するため採用しない。
- **リスク: 空 prompt＋画像なしで空 text block が残る**。これは Codex と対称の意図的挙動。
  frontend が本文・画像とも空の送信を通常許可しないため実運用上到達しにくい（requirements
  の仮定に準拠）。backend での送信拒否は本 Issue に含めない。

## 仮定

- 空 text block の生成箇所は `claude/wire.rs` の `user_message` のみであり、
  `claude/session.rs:212` はこれを無加工で呼ぶだけ（監査 OB-7・requirements と一致）。
- `user_message` のシグネチャは変更せず、関数内で `images` を一度 collect して
  空判定する（上記データモデル節の実装方針）。
- 空 prompt ＋ 画像なしのケースで空 text block を残す挙動は許容される（Codex と対称）。
- 「Anthropic API が空 text block を拒否し得る」挙動は非対称解消の根拠として扱い、
  CLI サニタイズ有無の外部検証は完了条件に含めない。
- image block の並び順は「text の後に image」を従来どおり維持する。

## Open Questions

なし
