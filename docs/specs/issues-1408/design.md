# Design

requirements.md / behavior.md を実装へ落とす設計。両 backend（Claude / Codex）の stdout 行読み取りを共通部品へ集約し、Codex の「非 JSON 行 1 行で即死」を解消する。Notice / ProtocolIncompatible への着地は本 ISSUE では扱わず（後続 Phase）、共通部品の導入・Codex 即死解消・可視化の対称化までを担う。

## 概要

- `infrastructure/agent_session/` に共通の stdout 行読み取り部品 `stdout_line_reader` を新設する。
- 部品は「1 行サイズ上限（共通定数 8MB）」「上限超過行の読み捨て」「非 JSON 行の分類」「破棄・skip 行の種別推定（probe）」を提供し、バイト境界で読み取る。
- Claude の既存 `next_stdout_item` / `read_stdout_line_limited` / `MAX_CLAUDE_STDOUT_LINE_BYTES` を共通部品へ移設し、挙動を回帰させない。
- Codex の `BufReader::lines()` 経路を共通部品へ置換し、非 JSON 行を「decode 失敗 → Fatal → セッション即死」から「skip＋カウント＋継続」へ変える。JSON-RPC 整合性が必要な例外条件のみ backend 側で失敗を伝搬する。
- 破棄（oversize）・skip の件数は Rust 側 runtime state に summary として持ち、構造化 warn ログとカウントで可視化する（full-retention を増やさない）。frontend は既存の read model 経由の表示のみ。

## 変更対象

- 新規: `src-tauri/src/infrastructure/agent_session/stdout_line_reader.rs`
- 変更: `src-tauri/src/infrastructure/agent_session/mod.rs`（`pub(crate) mod stdout_line_reader;` 追加）
- 変更: `src-tauri/src/infrastructure/agent_session/claude/process.rs`
  - `MAX_CLAUDE_STDOUT_LINE_BYTES` / `ClaudeStdoutItem` / `ClaudeStdoutLine` / `read_stdout_line_limited` / `next_stdout_item` を共通部品へ移設・置換。
- 変更: `src-tauri/src/infrastructure/agent_session/claude/session.rs`
  - 共通 `StdoutDiagnostics` を runtime state に保持し、backend 固有の event 写像だけを行う。
- 変更: `src-tauri/src/infrastructure/agent_session/codex/app_server.rs`
  - `Lines<BufReader<ChildStdout>>` を共通 reader へ置換。`next_json` の戻り値を共通 `StdoutItem` に変更。
- 変更: `src-tauri/src/infrastructure/agent_session/codex/session.rs`
  - 共通 `StdoutDiagnostics` を runtime state に保持する。`read_loop` に oversize/非 JSON 行の skip＋カウント＋継続分岐を追加し、共有 response tracker の検証失敗を伝搬する。
- 変更: `src-tauri/src/infrastructure/agent_session/codex/wire.rs`
  - 応答必須 request id と method の所有、response の `result` / `error` 排他検証を `PendingClientRequests` に集約する。
- 新規: fixture（配置は「テスト方針」参照）。

frontend の変更は行わない（可視化は既存 Rust-owned read model 経由）。

## アーキテクチャと責務分割

レイヤーは全て `infrastructure/`（外部プロセス I/O）に閉じる。domain / usecase の型は変更しない。

責務境界:

- **共通部品 `stdout_line_reader`（新設）**: バイト単位で 1 行を読み、サイズ上限を適用し、行を「JSON / 非 JSON / oversize」に分類して返す。加えて `StdoutDiagnostics` がカウンタ更新・reset・構造化 warn ログ・共通上限を参照した oversize 表示文言を一元化する。JSON-RPC 整合性と in-flight request 追跡は持たない。ジェネリック reader（`tokio::io::AsyncBufRead`）で駆動でき、fixture テスト可能にする。
- **backend 呼び出し層（`claude/*` `codex/*`）**: 共通部品の返す分類を受け、
  - 共通 diagnostics の runtime state への保持、
  - 共通文言から backend event（Error part 相当）への写像、
  - JSON-RPC 整合性の判断（Codex の例外条件）、
  を行う。Codex の in-flight request 追跡と `result` / `error` 排他検証は `codex/wire.rs` の `PendingClientRequests` に集約し、startup・turn・one-shot の全 request 経路で共有する（requirements 仮定に準拠し、共通 line reader には持ち込まない）。

R7（Notice 拡張点）は、共通部品が返す分類 enum（`StdoutItem`）と probe をそのまま後続の `Notice(OversizeDropped / UnsupportedMessage / Diagnostic)` へ写像できる形にしておくことで満たす。本 ISSUE では Notice へは接続せず、backend 側で既存の warn ログ＋Error part＋カウントに着地させる。

## データモデルまたは型

### 共通部品（`stdout_line_reader.rs`）

```rust
/// 両 backend 共通の 1 行サイズ上限。Claude 既存の 8MB を踏襲する。
pub(crate) const MAX_STDOUT_LINE_BYTES: usize = 8 * 1024 * 1024;

/// 破棄・skip 行の種別推定。巨大行全体は保持せず、先頭数バイトのみから推定する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineProbe {
    /// 先頭バイトから推定した種別（例: JSON の "type" / "method" 値、非 JSON の場合は None）。
    pub kind_hint: Option<String>,
    /// 破棄・skip した論理行のバイト数。
    pub bytes: usize,
}

/// 共通 reader が返す 1 件。
#[derive(Debug)]
pub(crate) enum StdoutItem {
    /// JSON としてパースできた行。
    Json(serde_json::Value),
    /// JSON としてパースできなかった行（skip 対象）。
    NonJson { probe: LineProbe },
    /// サイズ上限を超えて読み捨てた行。
    Oversize { probe: LineProbe },
}

pub(crate) struct StdoutLineReader<R> {
    inner: R,
}

impl<R: tokio::io::AsyncBufRead + Unpin> StdoutLineReader<R> {
    pub(crate) fn new(inner: R) -> Self { /* ... */ }
    /// 次の 1 行を分類して返す。EOF で None。I/O エラーのみ Err。
    pub(crate) async fn next(&mut self) -> Result<Option<StdoutItem>, String> { /* ... */ }
}

#[derive(Debug, Default)]
pub(crate) struct StdoutDiagnostics {
    oversize_dropped_count: u64,
    skipped_non_json_count: u64,
}
```

- `next()` は Claude 既存の `read_stdout_line_limited` を内包する。上限内の行は `serde_json::from_slice` で JSON 化し、成功なら `Json`、失敗なら `NonJson { probe }`。上限超過は `Oversize { probe }`。
- probe の種別推定（R6）: oversize 行は先頭を読み捨てる前に、先頭 N バイト（例: 先頭 4KB）だけを別途保持し、`serde_json` の部分パースではなく軽量スキャン（`"type":"..."` / `"method":"..."` のキーを先頭領域から正規表現的に探索、または先頭を切り出して緩く JSON パース試行）で `kind_hint` を得る。巨大行全体は保持しない（R9）。非 JSON 行の probe は行が上限内なので全体を持てるが、`kind_hint` は原則 None（JSON でないため）。
- **I/O エラーのみ** `Err` を返す。JSON パース失敗は `NonJson` であり `Err` にしない（これが Codex 即死解消の核）。

### Claude 側

- 既存 `ClaudeStdoutItem` は撤去し、`next_json()` は共通 `StdoutItem` を返す。`claude/session.rs::read_loop` を `Json` / `NonJson` / `Oversize` の 3 分岐へ更新。
- `ClaudeRuntimeState` に `stdout_diagnostics: StdoutDiagnostics` を追加し、既存の破棄件数と新しい skip 件数を同じ共通型で保持する。

### Codex 側

- `CodexAppServerProcess.stdout` を `Lines<BufReader<ChildStdout>>` から `StdoutLineReader<BufReader<ChildStdout>>` に変更。`next_json()` は共通 `StdoutItem` を返す。
- `CodexRuntimeState` に以下を追加:
  ```rust
  pending_client_requests: PendingClientRequests,
  stdout_diagnostics: StdoutDiagnostics,
  ```

## 処理フロー

### 共通 reader `next()`

1. `read_stdout_line_limited` 相当でバイト行を読む。
2. 上限超過なら残りを読み捨て、先頭保持分から probe を作り `Oversize { probe }`。
3. 上限内の行を `serde_json::from_slice`:
   - 成功 → `Json(value)`
   - 失敗 → probe（`kind_hint = None`）を付け `NonJson { probe }`
4. EOF は `None`。I/O エラーのみ `Err`。

### Claude `read_loop`

- `Json(value)` → 従来の `convert_claude_message` 経路（変更なし）。
- `Oversize { probe }` → `StdoutDiagnostics::record_oversize_drop` で count・warn・共通文言を生成し、Claude 固有の `MessagePart::Error` event へ写像する。**種別推定を文言へ付与**（R6、例: 「backend からの応答 1 件がサイズ上限（8MB）を超えたため破棄しました（推定種別: <kind_hint>）」。`kind_hint` が None なら種別注記を省く）。
- `NonJson { probe }` → `StdoutDiagnostics::record_non_json_skip` で warn ログ＋count を更新する（従来どおり継続。Error part は出さない）。
- crash 時の併記（`emit_crash_if_unexpected`）は既存どおり `oversize_dropped_count` を使う（回帰させない、behavior「累積破棄件数が併記される」）。

### Codex `read_loop`

`process.next_json().await` の結果分岐:

- `Ok(Some(StdoutItem::Json(message)))` → 従来の `message_kind` / `convert_jsonrpc_message` 経路。
  - **例外条件（R2 の例外）**: `message_kind` が `Response { id }` かつ `PendingClientRequests` に `id` が存在するのに、`result` / `error` が排他的に存在しない場合は、従来の失敗経路（`Fatal` + `TurnCompleted(Crash)`）を維持する。純粋な非 JSON 行は id を取り出せないため、この例外には該当しない。
- `Ok(Some(StdoutItem::NonJson { probe }))` → `skipped_non_json_count += 1`、warn ログ、**継続**（従来の Fatal を廃止）。応答待ちの有無に関わらず skip する（純粋な非 JSON 行は対応 request を特定できないため。behavior「応答を待っていない非 JSON 行は skip」を包含し、案 A に準拠）。
- `Ok(Some(StdoutItem::Oversize { probe }))` → `oversize_dropped_count += 1`、warn ログ、Claude と対称に `MessagePart::Error` 相当を送出、**継続**。
- `Ok(None)` / `Err(_)` → 従来どおり（プロセス終了 / I/O エラーは Crash）。

Codex の可視化も共通 `StdoutDiagnostics` を使い、生成された共通文言だけを backend event へ写像する。oversize は Error part＋count、非 JSON skip は warn＋count に着地させる（R5）。one-shot 経路も同じ diagnostics を使う。

## エラー処理

- **I/O エラー**（`fill_buf` 失敗）: 共通部品が `Err(String)` を返し、backend 側で従来どおり Crash 扱い。
- **JSON パース失敗**: `Err` にせず `NonJson` として分類。これにより Codex の即死を解消（R2）。
- **サイズ上限超過**: `Err` にせず `Oversize` として読み捨て。メモリ肥大を防ぐ（R3 / R9）。両 backend に同一上限が効く。
- **Codex 例外条件**: 応答必須の JSON-RPC response の整合性欠落のみ、backend 呼び出し層で失敗を伝搬（behavior「応答必須 request に対する非整合な response は失敗として扱われる」）。共通部品はこの判断を持たない。
- **改行なし EOF の上限超過**（behavior 明記）: Claude 既存ロジックのとおり `Oversize` として可視化し、後続なしで `None` に至る。共通部品へ移設しても回帰させない。

## テスト方針

### 共通部品の単体テスト（`stdout_line_reader.rs` 内 `#[cfg(test)]`）

`tokio::io::BufReader` にバイト列を流して駆動（Claude 既存テストの手法を踏襲）:

- 上限未満の JSON 行 → `Json`。
- 上限超過行 → `Oversize`、後続 JSON 行の処理継続（改行あり／改行なし EOF の両方）。
- 非 JSON 行 → `NonJson`、後続 JSON 行の処理継続。
- oversize probe の `kind_hint` が先頭バイトから推定される（R6）。
- I/O エラーが `Err` として伝搬する。

Claude 既存の `test_next_stdout_item_*`（`process.rs:485-537`）は共通部品テストへ移設し、意図的仕様を回帰させない（R4）。

### Claude backend テスト（`claude/session.rs`）

- 非 JSON 行 skip＋8MB 超破棄が混ざってもセッション継続、`oversize_dropped_count` カウント表示、Error part 可視化が維持される（R4、既存 `oversize_dropped_count` テストを維持・拡張）。
- 破棄時に probe の種別推定が可視化文言へ付与される（R6）。

### Codex backend テスト（`codex/session.rs`）

- 非 JSON 行を含む stdout でセッション継続、`skipped_non_json_count` がカウントされる（R2 / R5）。
- 非 JSON 行が複数連続しても継続（behavior）。
- 上限超過行が読み捨てられ後続処理継続、`oversize_dropped_count` カウント（R3）。
- 応答必須 request に対応する非整合 response の decode 失敗が従来どおり失敗として扱われる（R2 例外）。
- 応答待ちでない非 JSON 行は失敗にならず skip される（behavior）。

両 backend の production `read_loop` 自体を `AsyncBufRead` ジェネリックに保つ。fixture テストは writer を閉じない duplex stream の reader を注入し、プロセスを起動せずに実際の loop が後続 JSON event を処理して待機を継続することを検証する（外部プロセスをテストで実行しない方針に準拠）。

### fixture（R8）

- 内容: 非 JSON 行（警告ログ 1 行・複数行）＋上限超過行＋正常 JSON 行を混ぜた `.jsonl` バイト列。
- 配置: **F1（wire record/replay 回帰テスト基盤 #1383）は agent_session 向けに未整備**（現状 fixtures は `adaptor/gateway/workflow/fixtures` のみ）。したがって本 ISSUEでは代替配置として `src-tauri/tests/fixtures/agent_session/mixed_stdout_{claude,codex}.jsonl` に置き、`include_bytes!` で読み込んで両 backend の production `read_loop` を駆動する回帰テストにする。F1 整備後に同 fixture を F1 へ移送できるよう、fixture はプレーンなバイト列に留める。
- 固定内容: 両 backend でセッションが終了せず継続し、破棄・skip 件数がカウントされること。

### 品質ゲート

`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`。frontend 変更なしのため `pnpm` 系は既存が通ることのみ確認。

## リスクと代替案

- **リスク: Codex の JSON-RPC 整合性を壊す。** 非 JSON 行を一律 skip すると、本来失敗させるべき破損 response まで握り潰す懸念。→ `PendingClientRequests` が全 client request の id / method を所有し、対応する `Response{id}` の `result` / `error` 排他違反を startup・turn・one-shot で同じ失敗として扱う。純粋な非 JSON 行は id 不明で対応 request を特定できないため、この例外には該当しない（未応答のままなら既存の応答待ち timeout / crash 経路が働く）。
- **リスク: oversize probe の種別推定コスト。** 先頭数 KB のみの軽量スキャンに限定し、巨大行全体のパースはしない（R9）。推定不能なら `kind_hint = None`。
- **代替案 A（不採用）: Codex を Claude と同じ `ClaudeStdoutItem` に相乗り。** backend 固有名が漏れるため、backend 非依存の共通型 `StdoutItem` を新設する。
- **代替案 B（不採用）: 共通 line reader に JSON-RPC 整合性や backend event 生成まで持たせる。** backend 固有契約が共通 reader へ漏れるため不採用。共通化対象は分類に加えて diagnostics の count・reset・ログ・表示文言までとし、JSON-RPC tracker は `codex/wire.rs`、event 写像は各 backend に置く。
- **代替案 C（後続）: `ProtocolIncompatible` による fail-closed。** I10 の理想形だが Phase 0・依存なしの本 ISSUE では実装せず、S5 / 後続 ISSUE に委ねる（requirements 確定事項 Q1・案 A）。

## 仮定

- 共通部品の module 名は `stdout_line_reader`、共通定数名は `MAX_STDOUT_LINE_BYTES`（初期値 8MB、Claude 既存値を踏襲）。
- 共通部品はバイト境界で読み取り、UTF-8 変換は不要（JSON パースは `serde_json::from_slice`、非 JSON 行は skip するため文字列化しない）。Codex も `Lines`（文字列ベース）からバイトベースへ寄せる。
- Codex の「応答必須 response の整合失敗のみ失敗伝搬」は、`message_kind == Response{id}` かつ `PendingClientRequests` に `id` が残存し、`result` / `error` の排他条件を満たさないケースに限定する。純粋な非 JSON 行（id 不明）は常に skip＋継続とする（behavior の 2 シナリオ・案 A に整合）。
- 破棄・skip カウントは runtime state に `u64` の delta として持ち、巨大行・skip 行の本体は保持しない（R9）。
- 可視化は oversize を Error part 相当＋count、非 JSON skip を warn ログ＋count に着地させる。Codex も Claude と対称に oversize を Error part 化する（R5 の最低要件はログ＋count だが、UX 対称性のため Error part も出す）。
- fixture は F1 未整備のため `src-tauri/tests/fixtures/agent_session/` に代替配置し、F1 整備後に移送可能なプレーンバイト列とする。
- probe の種別推定は先頭数 KB の軽量スキャンで行い、巨大行全体は保持しない。

## Open Questions

なし。
