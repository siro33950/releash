# Design

本 Issue（#1249）の実装方針・責務分割・データ構造・処理フロー・エラー処理・テスト方針を定義する。
要求は `requirements.md`、観測可能な振る舞いは `behavior.md` を正本とする。本書はそれらを実コードへ落とし込むための設計詳細であり、確定値（閾値）の最終固定は #1209 の performance budget 確定後に行う（A3）。

正本ドキュメント: `docs/releash-performance-architecture-audit.md`（M2 / セクション 4「Agent Session Storage / Streaming」）

## 概要

巨大 tool output（test / lint / shell / tool result 由来）の**保存境界**を定義し、`MessagePart::ToolResult` が full 本文ではなく **`content_ref` / truncated `preview` / privacy-safe `summary`** を保持できるようにする。full output は message body とは別の **content-addressed blob store**（attachment store と同方式）へ退避し、message page / streaming delta / WS 配信のいずれも tool output 全長に比例して payload が増えない状態にする。frontend は full output を **invoke 経由で必要時のみ遅延取得**する。あわせて #1209 telemetry に truncated count / full output bytes の観測点を追加し、tool output 本文を通常ログ・span attribute・metric ラベルへ出さない。

設計の中心となる既存の「先行事例」は **attachment（画像）の externalize / hydrate 経路**である。Image → ImageRef への外部化（content-addressed blob + ref id）は本 Issue が tool output に対して行う退避とほぼ同型であり、その差分は「page read 時に hydrate しない（ref + preview のまま返す）」点に集約される。

## 変更対象

実コード調査（`Explore` agent による）で特定した変更対象を、層ごとに示す。行番号は調査時点のもの。

### usecase / domain 層（ロジックの正典 — `.claude/rules/rust-first-logic.md`）

- `src-tauri/src/usecase/agent_session/session/mod.rs`
  - `MessagePart::ToolResult`（行 89-）に ref / preview / summary を持てるよう拡張（後述「データモデル」）。
  - `ActivityEntry::ToolResult`（行 263-, legacy 表現）は本 Issue では本文退避の対象外（非スコープ: legacy 二重保持の全面廃止）。ただし legacy 経路で full 本文を再び書き戻さないことだけ担保する。
- `src-tauri/src/domain/agent_session/storage.rs`
  - `AgentSessionReader` trait に full output 取得メソッド（`get_tool_output` 等）を追加。
  - `AgentSessionStorageTypes` に full output 型（または既存 `Attachment` 型の再利用）を追加。
  - 既存 `get_session_attachment` / `get_session_page` / `persist_message_parts` のシグネチャは原則維持する。
- 新規 port: **`ToolOutputStore`**（domain 層 trait）。`AgentSessionReader`/`Writer` に統合するか独立 trait にするかは「アーキテクチャ」節で確定。

### adaptor 層（gateway 実装 / controller command）

- `src-tauri/src/adaptor/gateway/agent_session/session_storage/`
  - `attachment_blob.rs`（行 16- `attachment_id`, 37- `get_session_attachment`, 66- `externalize_message_attachments`, 110- `hydrate_message_attachments`）に倣い、tool output blob 用モジュール（`tool_output_blob.rs` 等）を新設。content-addressed id（SHA256）・blob 保存・取得を実装。
  - `message_store.rs`（externalize は persist 時 212/291/668 行、hydrate は read/page 時 90/627 行）に、tool output の externalize 呼び出しを追加。**画像と異なり page hydrate はしない**。
  - `layout.rs`（attachments ディレクトリ定義）に tool output blob ディレクトリを追加。
  - session 削除（`meta_repository.rs` 行 12- `remove_session_file_and_cache`）は `remove_dir_all` で session ディレクトリ全体を削除済み。tool output blob を session ディレクトリ配下に置けば**追加実装なしで retention（A4: session ライフサイクル連動）が満たされる**。
- `src-tauri/src/adaptor/controller/command/agent_session/session.rs`
  - full output 取得 Tauri command（`get_session_tool_output` 等）を新設。`get_session_attachment`（行 576-）と同型。
  - `command/mod.rs` の invoke handler 登録に追加。

### infrastructure / runtime 層（streaming 経路）

- `src-tauri/src/infrastructure/agent_session/runtime/bridge_common/sdk_message.rs`
  - `extract_tool_result_content`（行 98-）は SDK raw → 文字列化を担うが、**truncate 判定・退避は persist seam（storage 層）で行う**ため、ここは原則変更しない（full 文字列を一旦生成しても、外部化は storage 層で行う）。後述「処理フロー」で seam を確定。
- `bridge_common/session_persistence.rs`（`persist_streaming_parts` 行 24-）/ `stream_emit.rs`（`emit_streaming_delta` 行 63-）
  - delta に載せる parts を **ref + preview 化した part** にする経路を通す（後述）。

### protocol / WS 層

- `src-tauri/src/protocol/agent.rs`
  - `AgentStreamPartMsg::ToolResult`（行 81-175 の enum）に ref / preview / summary フィールドを追加。
  - `MessagePart → AgentStreamPartMsg` 変換（行 359-459 `impl From`）を更新。
- `src-tauri/src/ws_bridge.rs`（`WsBroadcaster`）は parts を運ぶだけで、part が ref + preview 化されていれば追加実装は不要。queue byte limit（512KB）等の既存 cap はそのまま機能する。

### telemetry 層

- `src-tauri/src/other/telemetry/`（または `infrastructure/telemetry/`）
  - truncated count（Counter）と full output bytes（Histogram または Counter）の metric を追加。退避が起きる seam（storage 層 externalize）で record する。

### frontend（インターフェースのみ）

- tool output part の表示で、ref を持つ場合に invoke（`get_session_tool_output`）で full output を遅延取得する経路を追加。**ロジックは持たず**、表示用フォーマットのみ（`.claude/rules/rust-first-logic.md`）。preview をデフォルト表示し、展開要求時に full を取得する最小経路に留める（UI デザイン新規は非スコープ）。

## アーキテクチャと責務分割

### 退避の seam（最重要設計判断）

tool output の truncate 判定・退避を**どこで行うか**には 2 候補がある。

- **候補 A（採用）: storage persist seam（adaptor gateway / `message_store.rs` の externalize）**
  画像の `externalize_message_attachments` と完全に対称。message を blob として永続化する直前に、`MessagePart::ToolResult` の `content` を閾値判定し、超過分を blob 退避して part を ref + preview + summary に書き換える。
  - 利点: 画像 externalize と同一 seam・同一テストパターン。session JSON（messages/*.json）に full 本文が残らないことを構造的に保証できる。retention（session ディレクトリ削除）も自動で満たす。
  - streaming delta については別途対応が必要（persist は 1000ms 間隔で、delta emit は 33ms 間隔のため、delta が persist より先に full 本文を運ぶ可能性がある）。→ 下記「streaming の扱い」で解決。
- 候補 B（不採用）: SDK message 受信時（`extract_tool_result_content`）で即退避。
  - 早期に ref 化できるが、storage 層と runtime 層の双方に退避ロジックが分散し、画像 externalize と非対称になる。不採用。

採用方針: **退避の正典は storage persist seam（候補 A）**。streaming delta はこの正典と整合させるため、emit に渡す part を「退避済み（ref + preview）の正規化済み part」へ projection してから載せる（下記）。

### streaming の扱い（delta / snapshot / resync）

delta emit（`stream_emit.rs`）は persist より高頻度で走るため、delta payload に full 本文が載らないことを別途保証する必要がある。方針:

- delta に載せる parts は、emit 直前に **truncate 判定を適用した projection 済み part**（ref がまだ確定していない段階では `content_ref` は未定でも、`preview` + `summary` で payload を bound する）にする。
- full output の blob 確定（content-addressed id 採番と blob 書き込み）は persist seam で行い、その id を delta の後続 seq または次回 snapshot / resync で part に反映する。
- resync / snapshot 経路（`resync_streaming_message_internal_with_data_dir`、`send_stream_snapshot`）も、退避済み part（ref + preview）を運ぶ。**full output は delta / snapshot 経路では決して運ばず、full output 取得経路（invoke）でのみ読む**（R6 / behavior「reconnect / resync でも full 全長を載せない」）。

設計上の不変条件（streaming で必ず守る）:
- 通常 delta・collapse 後 snapshot・resync のいずれの payload でも、1 つの tool output 由来 part が運ぶバイト量は `preview 上限 + summary 上限 + ref` で bound される（full output 全長に比例しない）。

> 注（仮定 ST1）: delta の preview は「閾値超過判定が確定した時点」で truncate して載せる。閾値判定が確定する前の極小 streaming chunk は inline で流れ得るが、各 chunk 自体が小さい（既存 256KB pending byte cap / 512KB queue byte cap で bound 済み）ため、full output 全長には比例しない。これにより「streaming 途中で 1 回も full を運ばない」ことを byte-bound として保証する。確定値は #1209 budget と整合させる。

### ToolOutputStore port の配置

- domain 層に trait を定義し、adaptor gateway（`FileSessionStorage` 系）が実装する。
- **採用: 既存 `AgentSessionReader` / `AgentSessionWriter` への統合**（独立 trait を新設しない）。
  - 理由: attachment が既に `get_session_attachment`（Reader）/ externalize（Writer 経由 persist）として同 trait に同居している。tool output も同じ blob 退避モデルであり、別 trait にすると Reader/Writer と二重に同じ `app_data_dir` / `session_id` 経路を引き回すことになる。`get_session_attachment` の隣に `get_session_tool_output`（仮）を置くのが最小差分。
  - requirements R3 の「`ToolOutputStore` port」は、この拡張された Reader/Writer の tool output 部分が論理的に果たす責務として満たす。命名上 `ToolOutputStore` という独立 trait を切るかどうかは実装時の整理に委ねるが、**依存方向（usecase → domain trait、adaptor が実装）は固定**する。

> 注（仮定 PORT1）: 上記は「最小差分・既存対称性」を優先した判断。もし将来 tool output store を attachment と独立に差し替えたい要件が出れば独立 trait に切り出すが、本 Issue では行わない（YAGNI）。

### 層と依存方向（確定事項）

```
frontend (React)
  │ invoke("get_session_tool_output")  ← 表示用フォーマットのみ
  ▼
adaptor/controller/command  (Tauri command)
  ▼
infrastructure/runtime      (streaming projection / persist seam 呼び出し)
  ▼
usecase/agent_session       (MessagePart / domain policy 呼び出し)
  ▼
domain/agent_session        (Reader/Writer trait + externalization policy)
  ▲ implements
adaptor/gateway/.../session_storage (blob 退避・取得の実装)
```

truncate 判定（閾値比較・preview/summary 生成）と attachment 検証のビジネスルールは domain の externalization policy に集約する。streaming projection（usecase/runtime）と persist seam（adaptor/gateway）は同じ domain policy に依存し、gateway が usecase の具体関数を import しない構成にする。blob の物理 I/O（content-addressed id 採番・ファイル read/write）は adaptor gateway に置く。

## データモデルまたは型

### MessagePart::ToolResult の拡張

現状（`session/mod.rs` 行 89-）:

```rust
ToolResult {
    content: String,                    // full 本文をそのまま保持
    #[serde(rename = "isError")]
    is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none", default, rename = "toolUseId")]
    tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default, rename = "parentToolUseId")]
    parent_tool_use_id: Option<String>,
}
```

拡張方針（後方互換を保つ追加フィールド方式）:

```rust
ToolResult {
    /// 閾値未満: full 本文を inline 保持（A5・従来どおり）。
    /// 閾値超過: 先頭一定量に truncate された preview のみを保持（full 全長を含まない）。
    content: String,
    #[serde(rename = "isError")]
    is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none", default, rename = "toolUseId")]
    tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default, rename = "parentToolUseId")]
    parent_tool_use_id: Option<String>,

    /// 退避済みの場合のみ Some。full output blob への content-addressed 参照。
    #[serde(skip_serializing_if = "Option::is_none", default, rename = "contentRef")]
    content_ref: Option<ToolOutputRef>,
    /// 退避済みの場合のみ Some。privacy-safe な集計メタデータ。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    summary: Option<ToolOutputSummary>,
}
```

設計上の取り決め:
- **truncate の表現**: 退避時は `content` フィールドそのものを「truncated preview」として再利用する（別フィールド `preview` を増やさない）。`content_ref.is_some()` であることが「これは preview であって full ではない」ことの判別になる。
  - これにより、ref を理解しない旧 frontend / 旧テストも `content`（= preview）をそのまま表示でき、後方互換が保たれる。
- 閾値未満: `content_ref = None`, `summary = None`, `content` = full inline（従来と完全に同一の serialize 結果）。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputRef {
    /// SHA256(content bytes).hex() — content-addressed（attachment_id と同方式）。
    pub id: String,
    /// full output の総バイト数（取得前のサイズ把握用 / privacy-safe）。
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputSummary {
    /// full output の総行数。
    pub line_count: u64,
    /// full output の総バイト数。
    pub byte_size: u64,
    /// tool 実行がエラーだったか（既存 is_error と同期）。
    pub is_error: bool,
    /// preview に含めた末尾が途中で切れているか等（任意・privacy-safe）。
    #[serde(default)]
    pub truncated: bool,
}
```

privacy 保証: `ToolOutputRef` / `ToolOutputSummary` はいずれも **集計値（id / 行数 / バイト数 / フラグ）のみ**で、full 本文の断片を含まない（preview は `content` 側に限定）。telemetry / log にはこれらの集計値すら本文として出さず、件数・総バイト数の集約のみ送る（R7）。

### protocol 型（`protocol/agent.rs`）

`AgentStreamPartMsg::ToolResult` にも上記と対応する `content_ref` / `summary`（camelCase serialize）を追加し、`AgentStreamAttachmentRefMsg`（id + media_type + byte_size）に倣った `ToolOutputRefMsg` を定義する。WS / Tauri 双方の delta・snapshot で同一形を運ぶ。

### blob の物理レイアウト

attachment（`{session}/attachments/{sha256}`）に倣い:

```
{app_data_dir}/sessions/{session_id}/
  ├── messages/{seq}.json        ← ToolResult は preview + ref のみ（full 本文なし）
  ├── attachments/{sha256}       ← 既存（画像）
  └── tool_outputs/{sha256}      ← 新規（full tool output blob）
```

- content-addressed（同一内容は dedup）。
- session ディレクトリ配下 → `remove_dir_all` による session 削除で自動 retention（A4）。
- A2 準拠: 新規 DB / 単一ファイル追記は採らない。

> 注（仮定 DEDUP1）: content-addressed dedup により、同一 session 内で同じ tool output が複数回出ても 1 blob で済む。**session をまたいだ dedup は行わない**（blob は session ディレクトリ配下に閉じる）。これにより session 削除時の broken reference（先行 Explore 報告が attachment で指摘した懸念）を構造的に回避する。

## 処理フロー

### 1. 保存（persist）時

```
SDK tool_result 受信
  → extract_tool_result_content で full 文字列化（runtime, 既存）
  → MessagePart::ToolResult { content: full, ... } を構築（既存）
  → persist 時に storage gateway の externalize 段で:
      if byte_size > MAX_BYTES || line_count > MAX_LINES:   // 閾値判定（usecase の純粋関数）
          id = sha256(full)
          write blob → tool_outputs/{id}
          part.content   = truncate_preview(full)           // 先頭一定量
          part.content_ref = Some(ToolOutputRef { id, byte_size })
          part.summary    = Some(ToolOutputSummary { line_count, byte_size, is_error, truncated:true })
          telemetry: truncated_count += 1; full_output_bytes += byte_size
      else:
          // 従来どおり inline（content=full, ref=None, summary=None）
  → messages/{seq}.json へ書き込み（full 本文は inline 分のみ）
```

### 2. message page 取得時

```
get_session_page (command → runtime → storage)
  → messages/{seq}.json を読む
  → ToolResult は preview + ref のまま返す（hydrate しない ← 画像との差分）
  → 小さい output は inline のまま返る
  → page payload は full output 全長に比例しない（R5）
```

### 3. full output 明示取得時

```
frontend: ref.id を指定して invoke("get_session_tool_output", { session_id, ref_id })
  → command → storage gateway: read tool_outputs/{ref_id}
  → full output 全長を返す（page / delta / snapshot とは独立経路, R3/R5）
```

### 4. streaming delta / resync 時

```
emit_streaming_delta (33ms 間隔)
  → 載せる parts を projection: 閾値超過 ToolResult は preview + summary(+ref 確定後は ref) に正規化
  → delta payload は full 全長を含まない（R6, 不変条件）
collapse → snapshot / resync
  → 同じく preview + ref のみ。full は full output 取得経路でのみ読む。
```

## エラー処理

各層のエラー型は既存方針（モジュール専用エラー / `Result<_, String>` の既存シグネチャ）に合わせる。

- **blob 書き込み失敗（persist 時）**: 退避に失敗した場合、part を ref 化せず **inline 本文のまま保存にフォールバック**する（データ欠落を防ぐ。session JSON は肥大化するが正しさを優先）。`log::warn!` で件数のみ記録し、**本文はログに出さない**（R7）。telemetry の truncated_count は成功時のみ加算。
- **blob 読み込み失敗（full output 取得時）**: `get_session_tool_output` は `Ok(None)`（blob 不在）/ `Err(String)`（I/O エラー）を返す。frontend は preview 表示を維持し、取得失敗を表示する（本文は出さない）。既存 `get_session_attachment` の `Result<Option<_>, String>` パターンに合わせる。
- **ref と blob の不整合（blob 削除済み等）**: content-addressed のため id 不一致は起き得ないが、blob 欠損時は `None` 扱い。preview は part に残るため最低限の情報は失われない。
- **session 削除との競合**: `remove_dir_all` 後の取得は `None`。例外を投げず None で返す。
- **streaming 中の persist 失敗**: 既存 `persist_streaming_parts` は失敗時 `false` を返し warn ログのみ（本文を出さないことを確認）。delta は preview 化済みのため、persist 失敗時も payload は bound されたまま。

## テスト方針

配置は CLAUDE.md 準拠（Rust: 各モジュール `#[cfg(test)]`、frontend: 同階層 `*.test.ts`）。本 Issue はロジックが Rust 集約のため、Rust 側単体・結合テストを主とする（A7: WS 受信クライアント不在のため E2E は行わず Rust テストで担保）。

### usecase 層（純粋ロジック）

- truncate 判定: 閾値ちょうど / 直前 / 直後（行数・バイト数それぞれ・両方）の境界値。
- preview 生成: 先頭一定量に切られ、full 全長を含まない。
- summary 生成: 行数 / バイト数 / is_error が正しく、本文断片を含まない。

### adaptor gateway 層（blob I/O）

- 閾値超過: blob が `tool_outputs/{id}` に書かれ、messages JSON の part は preview + ref のみ（full 本文なし）を assert（`attachment_blob.rs` の既存テスト 379/406 行パターンに倣う）。
- 閾値未満: blob 未生成・part は inline のまま。
- content-addressed: 同一内容 → 同一 id・1 blob（dedup）。
- full output 取得: ref id で full 全長が一意取得でき、page 経路とは独立。
- session 削除: `remove_session` 後に tool_outputs blob も消える（`meta_repository.rs` の `remove_dir_all` 経路）。
- 異常系: blob 書き込み失敗時の inline フォールバック、読み込み失敗時の None/Err。

### protocol / streaming 層

- `MessagePart → AgentStreamPartMsg` 変換で ref/summary/preview が保たれ full が載らない。
- delta / snapshot / resync payload が full 全長に比例しない（既存 `stream_emit.rs` のテスト計測ハーネス `TestMetricRecord` / payload bytes 計測を利用）。
- collapse 後 snapshot も ref + preview を運ぶ。

### telemetry 層

- 退避時に truncated_count / full_output_bytes が記録される（既存 `TestTelemetryGuard` / `TEST_METRIC_RECORDS` を利用）。
- log / span attribute / metric label に本文が出ない（出力文字列に preview/full の断片が含まれないことを assert）。

### 非退行 / privacy 検証（R8）

- 閾値超過 output を含む session で、session JSON サイズ・page payload・streaming frame payload が full 全長に比例しないことを bytes 計測で検証。
- full output が page 取得では読まれず、明示 invoke 時のみ読まれることを検証（読み込み回数 / 呼び出し痕跡で確認）。
- 既存 `cargo test` / `cargo clippy -D warnings` / `pnpm test` / `pnpm lint` が green。

## リスクと代替案

- **R-1 streaming で full を一度も運ばない保証の難しさ**: delta は persist より高頻度のため、閾値確定前の chunk が inline で流れる可能性がある。→ 各 chunk は既存 pending/queue の byte cap（256KB/512KB）で bound され、full 全長には比例しない（仮定 ST1）。閾値確定後は preview 化。確定値は #1209 budget と整合。代替案: SDK 受信時に即退避（候補 B）すれば streaming も常に ref 化できるが、退避ロジックが二層に分散し画像 externalize と非対称になるため不採用。
- **R-2 後方互換**: `content` を preview として再利用し、ref/summary を追加 optional フィールド（`skip_serializing_if`）にすることで、旧 session JSON / 旧 frontend と互換。既存 ToolResult を持つ session を読んでも `content_ref=None` で従来動作。
- **R-3 閾値の暫定性**: A3 により確定値は #1209 待ち。design では閾値を定数（`MAX_TOOL_OUTPUT_BYTES` / `MAX_TOOL_OUTPUT_LINES`）として 1 箇所に定義し、検討起点を OpenCode 相当（max ~1000 lines / ~30KB）とする。budget 確定後に定数差し替えのみで調整可能にする。
- **R-4 dedup と session 削除**: session をまたいだ dedup を行わない（blob を session ディレクトリ配下に閉じる）ことで broken reference を回避（仮定 DEDUP1）。同 session 内 dedup は維持。
- **R-5 ToolOutputStore を独立 trait にしない判断**: 将来の差し替え柔軟性は下がるが、本 Issue では既存 Reader/Writer 対称性・最小差分を優先（仮定 PORT1）。

## 仮定

requirements / behavior の確定仮定（A1〜A6）に加え、本設計で置く仮定:

- **ST1**: streaming delta は「閾値確定後 preview 化」＋「各 chunk は既存 byte cap で bound」により、full 全長を payload に比例させない。閾値確定前の極小 chunk が一時的に inline で流れることは許容する（byte-bound されるため R6 を破らない）。
- **PORT1**: `ToolOutputStore` の責務は既存 `AgentSessionReader`/`Writer` 拡張として実装し、独立 trait は新設しない（最小差分・YAGNI）。requirements R3 の port 要件は依存方向（usecase→domain trait、adaptor 実装）の固定で満たす。
- **DEDUP1**: full output blob は content-addressed だが session ディレクトリ配下に閉じ、session をまたいだ dedup は行わない（session 削除での broken reference 回避）。
- **PREVIEW1**: truncate 表現は新フィールドを増やさず `content` を preview として再利用し、`content_ref.is_some()` を「preview である」判別に使う（後方互換のため）。
- **THRESH1**: 閾値は定数 1 箇所定義、検討起点 OpenCode 相当（~1000 lines / ~30KB）、#1209 budget 確定後に確定値へ差し替え（A3）。

## Open Questions

なし（requirements の A1〜A6、および本設計の ST1 / PORT1 / DEDUP1 / PREVIEW1 / THRESH1 で確定。閾値の確定値のみ #1209 budget 確定後に実装で固定する旨を A3 / THRESH1 として明記済み）。
