# Requirements

## Type

read model 移行（責務境界の整理）。frontend helper が持つ diff / markdown source line / review display 用の read model 計算を Rust-owned query へ移し、frontend を backend read model の描画 adapter にする。外部から観測可能な振る舞い（描画結果・diff 表示・review anchoring の同一性）は原則不変とする。

対象 Issue: #1305（[impl] frontend diff/markdown read model migration）

種別: Implementation ISSUE（親 ISSUE ではない）。
マイルストーン: [12] クリーンアーキテクチャ移行

依存関係:

- Depends on: #1132（review comment / thread と diff line anchoring の境界を確定するため）。本 spec は #1132 完了後に着手する前提で書く。
- Blocks: #878（final dead-code sweep）。

## 背景と目的

### 背景

現状、diff / markdown 表示と review display に使う read model 計算が frontend helper に存在する（実コードで確認）。

- `src/lib/markdownDiff.ts`（176 行）— diff range / row / inline chunk の **domain calculation を frontend が所有** している。
  - `computeModifiedDiffRanges` / `computeOriginalDiffRanges`（行 11-96）: old/new content から `DiffRange { startLine, endLine, type }` を算出（jsdiff の `diffLines` 使用）。
  - `computeSplitRows`（行 104-145）: 左右分割表示用の行単位 split diff（`SplitRow { left, right, type }`）。
  - `computeInlineChunks`（行 152-175）: inline diff（`InlineChunk { content, type }`）。
- `src/lib/rehypeSourceLines.ts`（99 行）— diff range を入力に HAST node の position（line）と overlap 判定し、block tag に diff class（`md-diff-gutter-{type}` 等）を付与する rehype plugin。`rangesOverlap` / `findMatchingRange`（行 39-56）が line/range の対応付け（read-model identity 寄りの判定）を行い、`visitBlock` / `rehypeSourceLines`（行 58-98）が AST への class 付与（rendering）を行う。
- `src/lib/markdownUtils.ts`（7 行）— `isMarkdownFile`（拡張子集合 `md` / `mdx` / `markdown` による content kind 判定）。
- `src/components/panels/MarkdownDiffViewer.tsx`（344 行）— `GutterView` / `SplitView` / `InlineView` が上記 frontend helper を直接呼んで diff 表示を構成する（行 44-46 / 93-95 / 142-144）。一方 `DiffOnlyMarkdownView`（行 174-301）は既に backend へ `invoke("compute_visible_markdown_blocks", ...)`（行 188-192）しており、backend read model 利用の前例が存在する。

backend には既に diff / markdown read model の基盤がある（実コードで確認）。

- `src-tauri/src/domain/code/services/hunk.rs`（約 1,121 行）: hunk → change group 分割、hidden range / visible markdown block 計算、source line mapping、review anchor 用の stable ID（`hunk_id` / `group_id`、`group_identity_hash` による context 付き同一性）。
- `src-tauri/src/adaptor/gateway/code/diff_compute.rs`（約 229 行）: git2 `Patch::from_buffers` による diff buffer → `Hunk` 変換（git2 依存を隔離）。
- `src-tauri/src/usecase/code_query_service.rs` / `code_usecase.rs`: read model の orchestration（`compute_visible_markdown_blocks` 等）。
- `src-tauri/src/adaptor/controller/command/code/`: Tauri command 登録（`compute_visible_markdown_blocks` / `compute_hidden_ranges` 等）。

この配置では、CLAUDE.md / `.claude/rules/rust-first-logic.md` が定める「全てのロジックは Rust に置く / frontend は interface に徹する」「状態の所有者を明確にする」に反し、diff row / range / inline chunk / source line identity という read model 計算が frontend に残っている。これは #878（final dead-code sweep）が前提とする責務境界を満たさない。

### 目的

`markdownDiff.ts` の diff range / row / inline chunk 計算、および `rehypeSourceLines.ts` / `markdownUtils.ts` が持つ read-model identity 判定を、Rust-owned query（backend read model）へ移す。frontend は backend が返す diff result と line mapping を受け取り、描画と interaction（syntax highlight / React rendering / hover / focus / selection / display-only rendering option）に限定する。これにより diff / markdown の domain 判断が Rust 側に集約され、同じ backend-owned read model を Tauri・将来の client surface から再利用できる状態にする。本移行は描画結果・diff 表示・review anchoring の同一性を原則として変えない。

## スコープ

- **diff 計算の Rust 移行**
  - `markdownDiff.ts` の `computeModifiedDiffRanges` / `computeOriginalDiffRanges`（diff range）、`computeSplitRows`（split row）、`computeInlineChunks`（inline chunk）に相当する計算を Rust query（`domain/code` + `usecase` + `adaptor/controller/command/code`）へ移す。frontend からは `invoke` で結果を受け取る。
  - 既存 backend の diff/read model 基盤（`hunk.rs` / `diff_compute.rs` / `code_query_service`）に接続・拡張する形を優先し、frontend 独自の diff library 由来計算を解消する。
- **source line mapping / read-model identity の Rust 化**
  - `rehypeSourceLines.ts` が作っている line/range の対応付け（`rangesOverlap` / `findMatchingRange` による「どの block がどの diff range か」の identity 判定）が domain / read-model identity に当たる場合、Rust 側の line mapping を source of truth とし、frontend はその結果を用いて AST への class 付与（rendering）のみを行う。
  - comment anchoring に使う line/range identity は backend read model（`hunk_id` / `group_id` 等）由来とする。frontend に anchoring 用の line identity 計算を残さない。
- **content kind 判定の整理**
  - `markdownUtils.ts` の `isMarkdownFile` が review / read model 判定に影響する場合は Rust 側へ移す。純粋な表示条件にとどまる場合は UI helper として明確化し frontend に残す。
- **frontend の adapter 化**
  - `MarkdownDiffViewer.tsx`（`GutterView` / `SplitView` / `InlineView`）を、backend read model（diff range / split row / inline chunk / line mapping）を描画する adapter にする。
- **テスト**
  - Rust test が追加・削除・変更・空行・inline chunk・markdown source line mapping をカバーする。
  - frontend test は rendering / interaction（diffMode による表示切替、backend read model の描画反映、hover/focus/selection）を検証する。

## 非スコープ

- review comment storage migration 本体（#1132）。本 spec は #1132 で確定する review comment / diff line anchoring 境界を前提に接続するのみ。
- visual redesign（diff 表示の見た目・操作系の刷新）。
- markdown renderer の差し替え（remark / rehype 等のレンダリング基盤の置換）。
- diff 表示モード（gutter / split / inline / diff-only）の仕様追加・変更。本移行は計算の所有者を移すことに閉じる。
- 本移行と無関係な dead code 削除（#878 の final sweep は Blocks 先であり本 spec では行わない）。
- 外部から観測可能な振る舞いの変更（描画される diff 結果、review anchoring の同一性、`compute_visible_markdown_blocks` 等の既存 command の I/O 契約）。

## 要求事項

- R1: `markdownDiff.ts` の diff range / split row / inline chunk 計算が frontend から除去され、Rust query が source of truth として算出すること。frontend は `invoke` で結果を受け取り描画に用いること（`.claude/rules/rust-first-logic.md`）。
- R2: diff 計算ロジックが `domain/code`（純粋計算、infrastructure 非依存）に置かれ、orchestration が `usecase`、Tauri command wrapper が `adaptor/controller/command/code` に置かれ、git2 等の diff source へのアクセスは gateway に隔離されること。可能な限り既存の `hunk.rs` / `diff_compute.rs` / `code_query_service` を再利用・拡張する。
- R3: `rehypeSourceLines.ts` が作る line/range の identity 判定が read-model identity に当たる範囲では Rust 側 line mapping を source of truth とし、frontend は backend 由来の mapping を用いた AST class 付与（rendering）に限定されること。
- R4: comment anchoring に使う line/range identity が backend read model 由来であり、frontend に anchoring 用の domain identity 計算が残らないこと（#1132 で確定した境界に接続する）。
- R5: `markdownUtils.ts` の `isMarkdownFile` が、review / read model 判定に影響するなら Rust へ移され、純粋な表示条件にとどまるなら UI helper として明確化されること。
- R6: `MarkdownDiffViewer.tsx`（`GutterView` / `SplitView` / `InlineView`）が backend read model の描画 adapter となり、diff row / range / inline chunk の domain calculation を frontend に持たないこと。
- R7: 移行後、frontend（`src/lib` / `src/components` / `src/hooks`）に diff row / range / inline chunk の domain calculation が残っていないこと。
- R8: Rust test が追加・削除・変更・空行・inline chunk・markdown source line mapping を検証すること。
- R9: frontend test が rendering / interaction（diffMode 切替、backend read model の描画反映、hover/focus/selection 等）を検証すること。
- R10: 外部から観測可能な振る舞い（描画される diff 結果、review anchoring の同一性、既存 command の I/O 契約）が移行前後で変わらないこと。
- R11: `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 受け入れ基準の概要

- frontend に diff row / range / inline chunk の domain calculation が残っていない（R1 / R6 / R7）。
- diff 計算が `domain/code` + `usecase` + `adaptor` の層に置かれ、diff source アクセスが gateway に隔離されている（R2）。
- source line mapping の read-model identity が Rust 由来で、frontend は rendering に限定されている（R3）。
- comment anchoring の line identity が backend read model 由来である（R4）。
- `isMarkdownFile` が判定の性質に応じて Rust / UI helper のいずれかへ正しく配置されている（R5）。
- Rust test が追加・削除・変更・空行・inline chunk・markdown source line mapping をカバーする（R8）。
- frontend test が rendering / interaction を検証する（R9）。
- 外部観測可能な振る舞いが移行前後で不変である（R10）。
- 上記すべての lint / test / build コマンドが通る（R11）。

詳細な受け入れシナリオ（Gherkin）は `behavior.md` で定義する。外部観測可能なビジネスルールに絞り、層配置や経路詳細は持ち込まない（[[feedback_behavior_definition_granularity]]）。

## 仮定

- A1: spec ディレクトリ名は `docs/specs/issues-1305` とする（直近 Issue の命名規約 `issues-NNN` に合わせる）。
- A2: 本移行は read model 計算の所有者を frontend から Rust へ移すものであり、描画される diff 結果・review anchoring の同一性・既存 Tauri command の I/O 契約を変えない。表現刷新やモード追加が必要になっても本 Issue では行わず、必要なら別 Issue 化する（外部観測可能な振る舞いを変えない方針: [[feedback_behavior_definition_granularity]]）。
- A3: backend には既に diff / markdown read model 基盤（`domain/code/services/hunk.rs`・`adaptor/gateway/code/diff_compute.rs`・`usecase/code_query_service.rs`・`adaptor/controller/command/code/`）が存在し、`compute_visible_markdown_blocks` 等で利用実績がある。本移行は新規モジュールを別途作るより、この既存基盤への接続・拡張を優先する。frontend の jsdiff 由来計算（行単位の split / inline）を backend のどの query 形（既存 hunk read model の拡張か新規 query 追加か）で満たすかは design.md で確定する。
- A4: 追加・変更する Tauri command / read model DTO の具体形（diff range / split row / inline chunk / line mapping の返却スキーマ、command 名）は design.md で確定する。requirements では「frontend が backend read model を受け取り描画に徹する」ことを要求に留める。
- A5: `rehypeSourceLines.ts` の `rangesOverlap` / `findMatchingRange` は、入力 diff range が backend 由来になることを前提に、AST node position との突合（rendering 都合の対応付け）として frontend に残すか、line mapping identity として Rust へ移すかを design.md で判定する。判定の原則は「read-model identity（comment anchoring・review 表示の同一性に効くもの）は Rust、AST rendering 都合の突合は frontend」とする。
- A6: comment anchoring の line identity は #1132 で確定する review comment / diff line anchoring 境界に従う。調査時点では frontend 側に anchoring 用 identity 計算は確認されず、backend の `hunk_id` / `group_id`（`group_identity_hash`）が anchor 同一性を担っている。本 Issue では「frontend に anchoring identity 計算を残さない／backend 由来である」ことの確認・接続を行い、anchoring 仕様自体は変更しない。
- A7: `isMarkdownFile` は、調査時点では表示分岐（markdown 専用ビューの選択）に用いられる表示条件と見られる。review / read model 判定に影響しないことを実装時に確認し、影響しなければ UI helper として明確化して frontend に残す。影響する場合は Rust へ移す。
- A8: 既存テスト（`markdownDiff` 等の frontend test、`hunk.rs` の Rust test）は対応する層へ移設・拡張しつつ R10（非退行）の回帰検証としても用いる。テストの期待値は実装に合わせて変更しない（仕様が正、実装が誤りなら実装を直す）。
- A9: 検証は CI と同じコマンド（`pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）で行う。

## Open Questions

なし。
