# Design

対象 Issue: #1305（[impl] frontend diff/markdown read model migration）

本書は `requirements.md` / `behavior.md` を前提に、frontend が所有する diff / markdown read model 計算を Rust-owned query へ移すための実装設計を定める。外部から観測可能な振る舞い（描画される diff 結果・review anchoring の同一性・既存 command の I/O 契約）は維持する（R10）。ただし R10 の「移行前後で同一」は、**jsdiff 出力とのバイト単位一致ではなく、behavior.md が定める分類規則（added / deleted / modified の判定と行対応）の同一性**として扱う。diff 計算は既存 backend の git2 基盤に置き換えるため、グルーピングの微差は behavior 規則を満たす範囲で許容する（決定 D1。ユーザー合意済み: 「最も正しく最もシンプルな Rust 実装」を優先し、厳密同一性は要件としない）。

## 概要

現状、markdown diff 表示の read model 計算が frontend helper に存在する。

- `src/lib/markdownDiff.ts`: `computeModifiedDiffRanges` / `computeOriginalDiffRanges`（diff range）、`computeSplitRows`（split row）、`computeInlineChunks`（inline chunk）。いずれも `diff`（jsdiff）の `diffLines` を用いた **行単位 domain 計算**。
- `src/lib/rehypeSourceLines.ts`: backend/frontend いずれかが算出した `DiffRange[]` を入力に、HAST block node の position(line) と overlap 判定して diff class を付与する rehype plugin。
- `src/lib/markdownUtils.ts`: `isMarkdownFile`（拡張子による content kind 判定）。

本設計では、`markdownDiff.ts` の 4 関数に相当する計算を Rust query へ移し、frontend（`MarkdownDiffViewer.tsx`）を backend read model の描画 adapter にする。`rehypeSourceLines.ts` は「backend 由来の DiffRange を入力に AST へ class 付与する rendering glue」として frontend に残す。`isMarkdownFile` は表示分岐専用の UI helper として frontend に残す（後述の判定根拠あり）。

backend には既に diff / markdown read model 基盤（`domain/code` + `usecase/code_query_service` + `adaptor/controller/command/code` + `adaptor/gateway/code/diff_compute.rs`）があり、`compute_visible_markdown_blocks` 等で利用実績がある。本移行はこの層構成（pure domain service → query service orchestration → DTO → Tauri command）を踏襲し、新しい read model（diff range / split row / inline chunk）を追加する。

## 変更対象

### 追加（Rust）

- `src-tauri/src/domain/code/value_objects/`（既存 `hunk.rs` / `range.rs` と同階層）
  - `DiffRange { start_line, end_line, kind: DiffRangeKind }`、`DiffRangeKind { Added, Modified, Deleted }`
  - `SplitRow { left: Option<String>, right: Option<String>, kind: SplitRowKind }`、`SplitRowKind { Unchanged, Added, Removed, Modified }`
  - `InlineChunk { content: String, kind: InlineChunkKind }`、`InlineChunkKind { Unchanged, Added, Removed }`
  - `DiffSide { Modified, Original }`（diff range 算出の対象側）
- `src-tauri/src/domain/code/services/` に markdown diff 計算の pure service
  - 入力 `Vec<Hunk>` と original / modified 全文から diff range（side 別）/ split row / inline chunk を導出する純粋関数群。infrastructure 非依存（R2）。`Hunk` は既存の domain VO であり、git2 への依存は gateway（`diff_compute.rs`）に隔離されたまま。
  - diff の算出には既存 git2 基盤（`adaptor/gateway/code/diff_compute.rs` の `diff_buffers` / usecase の `compute_diff_hunks`）を再利用する（決定 D1）。新規 diff crate は追加しない。
  - hunk は変更領域＋context のみを保持するため、split / inline が要する「unchanged を含む全文 block 列」は、hunk 間の gap を original / modified 全文から行スライスで補って組み立てる。この hunk＋全文の復元パターンは既存 `compute_visible_markdown_blocks` と同型。
- `src-tauri/src/usecase/code_dto.rs`
  - `DiffRangeDto` / `SplitRowDto` / `InlineChunkDto`（serde camelCase。frontend が現在使う JSON 形に一致させる。下記「データモデル」参照）と VO→DTO 変換関数。
- `src-tauri/src/usecase/code_query_service.rs`
  - `compute_markdown_diff_ranges(original, modified, side) -> Vec<DiffRangeDto>`
  - `compute_markdown_split_rows(original, modified) -> Vec<SplitRowDto>`
  - `compute_markdown_inline_chunks(original, modified) -> Vec<InlineChunkDto>`
- `src-tauri/src/usecase/code_usecase.rs`
  - 上記 3 メソッドへの委譲。
- `src-tauri/src/adaptor/controller/command/code/`（`hunk.rs` 近傍、もしくは `markdown.rs` を新設）
  - Tauri command 3 本を追加し、`mod.rs` の `COMMAND_NAMES` と `invoke_handler()`（`generate_handler!`）へ登録する。

### 変更（frontend）

- `src/components/panels/MarkdownDiffViewer.tsx`
  - `GutterView` / `SplitView` / `InlineView` が `computeModifiedDiffRanges` / `computeSplitRows` / `computeInlineChunks` を直接呼ぶのをやめ、`invoke` で backend read model を取得する。取得は `DiffOnlyMarkdownView` 既存の `useEffect` + cancel パターンに揃える。
- `src/lib/rehypeSourceLines.ts`
  - 入力 `DiffRange[]` の型 import 元を `markdownDiff.ts` から、frontend-facing type（`src/types/` もしくは backend DTO に対応する型定義）へ変更する。plugin ロジック自体（overlap 判定 → class 付与）は rendering glue として維持する。

### 削除（frontend）

- `src/lib/markdownDiff.ts`（4 計算関数と型）を削除する。型 `DiffRange` / `SplitRow` / `InlineChunk` は frontend-facing type 定義（backend DTO に対応）へ移設する。
- `src/lib/__tests__/markdownDiff.test.ts` を削除し、その検証内容（追加/削除/変更/空行/inline/mixed/空入力）を Rust 側の golden test として移設する（A8）。

### 据え置き（変更しない）

- `compute_visible_markdown_blocks` / `compute_hidden_ranges` 系 command の I/O 契約（R10、非スコープ）。`DiffOnlyMarkdownView` は現状のまま。
- `src/lib/markdownUtils.ts`（`isMarkdownFile`）と `src/lib/markdownUtils.test.ts`。後述の理由で UI helper として frontend に残す。
- review comment / diff line anchoring の仕様（#1132 で確定。本 Issue では接続・確認のみ）。

## アーキテクチャと責務分割

```
原文(original) / 変更後(modified)  ── invoke ──▶  Tauri command (adaptor/controller/command/code)
                                                      │ 入力受領・usecase 呼び出しのみ
                                                      ▼
                                                 code_usecase → code_query_service (usecase)
                                                      │ orchestration・VO→DTO 変換
                                                      │ ① diff_buffers/compute_diff_hunks で hunk 取得（git2, gateway）
                                                      ▼
                                                 markdown diff service (domain/code/services)
                                                      │ pure: hunk＋全文 → range/row/chunk 導出
                                                      ▼
                                                 DiffRange / SplitRow / InlineChunk (VO)
                                                      ▲
                                                      └ DTO 化して返却
frontend (MarkdownDiffViewer)  ◀── read model ──┘
   GutterView   : DiffRangeDto[] → rehypeSourceLines → AST class 付与（rendering）
   SplitView    : SplitRowDto[]  → grid 描画（rendering）
   InlineView   : InlineChunkDto[] → 連なり描画（rendering）
```

- **domain（pure）**: hunk（gateway 由来）＋全文からの range/row/chunk 導出。behavior.md の分類規則（後述）を満たすように写像する。infrastructure 非依存（R2）。
- **usecase**: original / modified を受け、gateway の `diff_buffers`（既存 `compute_diff_hunks` 経由）で hunk を得てから domain service を呼び、VO を DTO へ変換して返す orchestration。git2 への依存は gateway に隔離。
- **adaptor/controller/command**: Tauri 入力を usecase 呼び出しへ変換するのみ。business behavior を持たない。
- **frontend**: backend read model を受け取り、syntax highlight / React rendering / AST class 付与 / hover / focus / selection に限定。diff の domain 判断を持たない（R1/R6/R7）。

### rehypeSourceLines の配置判断（R3 / A5）

`rangesOverlap` / `findMatchingRange` は **frontend が remark/rehype で生成した HAST node の position(line) と DiffRange を突合**する処理である。突合対象（AST node position）は frontend rendering の産物であり、backend が所有しない。よって「read-model identity は Rust、AST rendering 都合の突合は frontend」の原則（A5）に従い、**plugin は frontend に残す**。ただし入力の `DiffRange`（どの行がどの種別か、という read-model identity）は backend 由来とする。これにより identity の source of truth は Rust に移り、frontend は識別済み range を AST へ反映する rendering に限定される（R3 を満たす）。

### isMarkdownFile の配置判断（R5 / A7）

`isMarkdownFile` の唯一の production 呼び出し元は `ReviewPanel.tsx:302` で、`isTextDiff && selectedFile ? isMarkdownFile(selectedFile) : false` として **markdown 専用ビューを出すかどうかの表示分岐**にのみ用いられている。review comment や read model の identity 判定には関与しない。よって純粋な表示条件であり、UI helper として frontend に残す（R5 の「純粋な表示条件にとどまる場合」に該当）。実装時にこの前提（review / read model 判定に影響しない）を改めて確認する。content kind 判定の結果同一性は既存 `markdownUtils.test.ts` で担保され、behavior の content kind Scenario（配置先に依らず結果同一）も満たす。

### comment anchoring（R4 / A6）

調査時点で frontend 側に anchoring 用の独自 line identity 計算は存在しない（`markdownDiff.ts` の range は gutter class 付与にのみ使われ、comment anchor には使われていない）。anchor 同一性は backend の `hunk_id` / `group_id`（`group_identity_hash`）が担う。本 Issue では「frontend に anchoring identity 計算を残さない／backend 由来である」ことを確認・維持するのみで、anchoring 仕様自体は変更しない。実装時に DiffComment 系 component が backend ID を anchor に用いていることを確認する。

## データモデルまたは型

frontend が現在消費している JSON 形と一致させ、I/O 契約を変えない（R10）。DTO は serde camelCase で serialize する。

### DiffRange

```rust
// domain VO
pub struct DiffRange { pub start_line: u32, pub end_line: u32, pub kind: DiffRangeKind }
pub enum DiffRangeKind { Added, Modified, Deleted }

// DTO (serialize)
// { "startLine": u32, "endLine": u32, "type": "added" | "modified" | "deleted" }
```

`kind` は JSON 上で `type` として serialize する（現行 frontend の `DiffRange.type` に一致。`#[serde(rename = "type")]`、値は lower-case）。1-based・endLine inclusive を維持する。

### SplitRow

```rust
pub struct SplitRow { pub left: Option<String>, pub right: Option<String>, pub kind: SplitRowKind }
pub enum SplitRowKind { Unchanged, Added, Removed, Modified }

// { "left": string | null, "right": string | null, "type": "unchanged" | "added" | "removed" | "modified" }
```

`left` / `right` は変更ブロックの **複数行を含む文字列**（行末 `\n` を含む。現行 `computeSplitRows` の挙動と一致）。`null` は片側欠落（added は left=null、removed は right=null）。

### InlineChunk

```rust
pub struct InlineChunk { pub content: String, pub kind: InlineChunkKind }
pub enum InlineChunkKind { Unchanged, Added, Removed }

// { "content": string, "type": "unchanged" | "added" | "removed" }
```

### Tauri command 入出力

| command | 引数 | 戻り値 |
| --- | --- | --- |
| `compute_markdown_diff_ranges` | `original: String, modified: String, side: "modified" \| "original"` | `Vec<DiffRangeDto>` |
| `compute_markdown_split_rows` | `original: String, modified: String` | `Vec<SplitRowDto>` |
| `compute_markdown_inline_chunks` | `original: String, modified: String` | `Vec<InlineChunkDto>` |

`side` 引数は `computeModifiedDiffRanges` / `computeOriginalDiffRanges` の両対応のため設ける。production の呼び出し元は GutterView（modified 側）のみだが、golden 同一性検証のため original 側も query として提供する（`computeOriginalDiffRanges` は現状 production 呼び出し元を持たず test のみだが、Rust 側で両側を網羅して golden を移設する）。

frontend-facing type（`src/types/markdownDiff.ts` 等、命名は実装時に確定）に `DiffRange` / `SplitRow` / `InlineChunk` を上記 JSON 形で定義し、`MarkdownDiffViewer.tsx` / `rehypeSourceLines.ts` から参照する。

## 処理フロー

### 行単位 diff の分類規則

`markdownDiff.ts` 現行実装と behavior.md の分類規則を Rust で実現する。これが移行前後の振る舞い同一性（R10 = 分類規則の同一性）の核。

- 行 diff は既存 git2 基盤（`diff_buffers` → `Vec<Hunk>`）で算出する。各 `Hunk` の `lines`（`+/-/ ` prefix）と `old_start` / `new_start` から、変更領域を `added`（`+` のみ）/ `deleted`（`-` のみ）/ `modified`（`-` 群の直後に `+` 群が隣接）へ分類する。hunk 間の gap は unchanged 領域として original / modified 全文から補い、全文を順序付きの change block 列（各 block は `unchanged | added | removed` と連続行をまとめた value を持つ）へ集約する。
- **diff range（modified 側）**: modified 側の論理行番号を 1 から進めながら、
  - removed の直後が added → `modified`（範囲は added 行数で算出、added 分だけ行を進める）
  - added 単独 → `added`
  - removed 単独 → modified 側には range を作らない
  - unchanged → 行を進めるのみ
- **diff range（original 側）**: original 側行番号を進めながら、
  - removed の直後が added → `modified`（範囲は removed 行数で算出）
  - removed 単独 → `deleted`
  - added 単独 → original 側には range を作らない
- **split row**: change block を 1 行（=1 SplitRow）に対応。removed+added 隣接 → `modified`（left=removed value, right=added value）、removed 単独 → `removed`、added 単独 → `added`、unchanged → `unchanged`（left=right=value）。
- **inline chunk**: change block を順に `unchanged | added | removed` の chunk へ写す。

### エッジケース（golden で固定）

現行実装の以下の挙動を厳密に維持する（既存 frontend test が根拠。Rust golden test へ移設）。

- `original === modified`: diff range は `[]`。split row は `original` が空なら `[]`、非空なら `[{left, right, unchanged}]` 1 件。inline chunk も同様に空なら `[]`、非空なら unchanged 1 件。
- `original` 空 / `modified` 空: 上記分類規則どおり（例: modified 空 → modified-side range は `[]`、split は removed 行、inline は removed chunk）。
- 行数の異なる removed/added 隣接でも 1 つの `modified` row/range として束ねる（jsdiff の block 粒度）。
- 末尾空行のみ追加 / 中間空行のみ削除 / 1 行置換 / 複数独立変更（mixed）が behavior Examples どおりに分類される。

### frontend データ取得フロー

各 View は `originalContent` / `modifiedContent`（`useDeferredValue` 済み）の変化で `invoke` し、結果を state に保持して描画する。`DiffOnlyMarkdownView` 既存の `let cancelled` ガード + `.catch` フォールバックパターンを踏襲する。`useMemo` による同期計算は廃し、非同期取得 + ローディング/フォールバック描画に置き換える。

## エラー処理

- Tauri command は `Result<_, AppError>` を返す（既存 `compute_visible_markdown_blocks` 等と同様）。domain service は panic せず、空入力・巨大入力でも有効な read model（多くは空 Vec）を返す純粋関数とする。
- frontend は `invoke().catch()` で失敗時に空の read model（`[]`）へフォールバックし、描画を破綻させない（`DiffOnlyMarkdownView` と同方針）。失敗時は変更ハイライト無しの素の描画になる。
- module ごとの専用 error type 方針に従い、新規ロジックが既存の `code` 系 error に収まらない場合のみ拡張する。通常はバリデーション不要（任意の文字列対を受ける）。

## テスト方針

### Rust（R8）

- domain service の unit test（該当 module 内 `#[cfg(test)] mod tests`）。
  - diff range（modified / original 両側）: 追加 / 削除 / 変更（removed+added 隣接）/ 複数行 / mixed / 空入力 / 同一入力。
  - split row: unchanged / added / removed / modified / 空・同一入力。
  - inline chunk: unchanged / added / removed / mixed / 空・同一入力。
  - 境界条件（behavior Examples）: 末尾空行追加 / 中間空行削除 / 1 行置換 / 複数独立変更。行番号対応のずれが無いこと。
  - markdown source line mapping: diff range が markdown 行に対し正しい行範囲を返すこと。
  - 検証基準は behavior.md の分類規則（added / deleted / modified の判定・行対応）とし、`markdownDiff.test.ts` のケースは分類が一致するシナリオ集として移設する（A8）。git2 のグルーピングが jsdiff と微差を生じる場合、behavior 規則を満たす範囲で期待値を Rust 実装の出力に合わせてよい（決定 D1。厳密同一性は要件としない）。ただし behavior の Rule / Examples（空入力・同一入力・modified 隣接・境界条件）は必ず満たすこと。
- usecase / DTO test: command 委譲の正常系と、DTO serialize 表現の golden（`code_dto.rs` 既存 DTO test と同様に JSON 形を固定し I/O 契約を守る）。

### frontend（R9）

- `MarkdownDiffViewer.tsx` の test を新設。`@tauri-apps/api` の `invoke` を `vi.mock` し、各 command が DTO を返す前提で:
  - diffMode（gutter / split / inline / diff-only）切替で対応 View が描画されること。
  - backend read model（range / row / chunk）が描画へ反映されること（split cell class、inline class、gutter class）。
  - hover / focus / selection 等の interaction（既存挙動があれば）。
  - 取得失敗時に空フォールバック描画になること。
- `rehypeSourceLines.test.ts` は維持（入力 DiffRange の型 import 元のみ追従）。plugin の overlap→class 付与の挙動は frontend 責務として継続検証。
- `markdownUtils.test.ts` は維持（`isMarkdownFile` を UI helper として残すため）。

### 非退行（R10）

- behavior.md の分類規則を基準とした Rust test により、range / row / chunk の振る舞い同一性を固定（バイト単位の jsdiff 一致ではなく分類規則の同一性。決定 D1）。`compute_visible_markdown_blocks` 等既存 command の I/O は変更しないため別途回帰不要。

### コマンド（R11）

`pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を通す。

## リスクと代替案

- **diff エンジンの選択（決定済み D1）**: 現行 frontend は jsdiff（`diffLines`）。Rust 側は既存 backend の git2 基盤（`diff_buffers` / `compute_diff_hunks`）を再利用する。新規 diff crate（`similar` 等）は追加しない。
  - 採用理由: (1) 新規依存ゼロ、(2) アプリ内の diff エンジンを git2 に一本化できる（diff-only ビューは既に git2。read model の所有者を1つに集約する本 Issue の目的に一致）、(3) R2「既存の `hunk.rs` / `diff_compute.rs` を再利用・拡張する」に適合、(4) 厳密同一性が不要（D1）になったため、`similar` で得ていた jsdiff 近似のメリットが薄れた。
  - トレードオフ: split / inline が要する全文 block 列を hunk＋全文から復元する導出が必要。ただし既存 `compute_visible_markdown_blocks` が同型の復元を実装済みで、煩雑さは限定的。
  - 不採用案: `similar`（pure Rust だが新規依存＋第2の diff エンジンを生む）、`imara-diff`（低レベルで過剰）、`dissimilar`（char 単位で行ブロックに不適）。
- **jsdiff とのグルーピング微差**: git2 と jsdiff で modified ブロックの粒度・tie-break が一致しない入力がありうる。R10 はバイト一致ではなく behavior 分類規則の同一性として扱う（D1）ため、behavior の Rule / Examples を満たす限り許容する。テストは behavior 規則を基準にする。
- **非同期化による描画ちらつき**: `useMemo` 同期計算から `invoke` 非同期取得へ変わるため、初回描画で一瞬ハイライト無しになりうる。`useDeferredValue` 継続使用と cancel ガードで緩和。視覚仕様の変更ではない。
- **frontend type の置き場所**: `DiffRange` 型を `markdownDiff.ts` から frontend-facing type へ移す際、`rehypeSourceLines.ts` の import 追従漏れに注意（型のみ参照のため lint/build で検知可能）。

## 仮定

- A1: 追加 command は content（original / modified 文字列）を入力とする content-based query とし、既存 `compute_visible_markdown_blocks` の入力様式に揃える。hunk-input 版は設けない。
- A2: DTO の JSON 形は現行 frontend が消費する形（`type` / `left` / `right` / `content` / `startLine` / `endLine`）に一致させ、I/O 契約を変えない。
- A3: `rehypeSourceLines.ts`（overlap→class 付与）は AST rendering glue として frontend に残す。入力 DiffRange は backend 由来。
- A4: `isMarkdownFile` は表示分岐専用の UI helper として frontend に残す（review / read model 判定に影響しないことを実装時に再確認）。
- A5: `computeOriginalDiffRanges` は現状 production 呼び出し元を持たないが、golden 移設と side 両対応のため Rust query では original 側も提供する。
- A6: テストの基準は behavior.md の分類規則（added / deleted / modified の判定・行対応）。`markdownDiff.test.ts` のケースは分類シナリオとして移設し、git2 のグルーピング微差が behavior 規則を満たす範囲では期待値を Rust 実装出力に合わせてよい（D1）。behavior の Rule / Examples は必ず満たす。
- A7: command 登録は既存定型（`#[tauri::command]` + `COMMAND_NAMES` + `generate_handler!`）に従う。

## 決定事項

- D1: Rust の行 diff エンジンは既存 backend の git2 基盤（`diff_buffers` / `compute_diff_hunks`）を再利用する。新規 diff crate は追加しない。R10 は「jsdiff とのバイト単位一致」ではなく「behavior.md の分類規則の同一性」として扱い、グルーピングの微差は behavior 規則を満たす範囲で許容する（ユーザー合意: 最もシンプルで正しい Rust 実装を優先・厳密同一性は不要・既存基盤再利用で diff エンジンを一本化）。

## Open Questions

なし。
