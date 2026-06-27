# Requirements

## Type

リファクタリング / 境界整理（frontend helper の責務分類。clean architecture 移行 milestone [12] の一部）。

関連: #1307 / #767（superseded）/ #878（final dead-code sweep, Blocks）/ #1132（comment/thread read model）

## 背景と目的

clean architecture 移行（milestone [12]）の方針は「全てのアプリケーションロジックを Rust が所有し、frontend は表示・入力・invoke・表示用フォーマットに徹する」ことである。

先行の #767 は frontend-owned helper を Rust へ移すことを目的としていたが、**UI-only helper まで移行対象に混ぜていた**ため、何を残し何を移すかの境界が不明確だった。#767 は superseded となり、責務分離の作業は粒度別 ISSUE に分割された。

本 ISSUE（#1307）は、その分割のうち **「残すべき UI-only helper と、Rust へ移すべき application/domain decision を明確に分離する」境界整理**を担当する。具体的には、#767 に列挙されていた frontend helper のうち、他の分割 ISSUE（branch/path、repository status、agent stream/state、diff read model、terminal/PTY）に属さない残余 helper を 1 つずつ分類し、

- UI-only helper として残すものは、その理由が**名前・配置・test から判別できる**状態にする。
- application/domain decision を含むものは、Rust command/usecase/read model 側へ移す。

本 ISSUE のゴールは、`#878 final dead-code sweep` を実施する前に「未分類の残余 helper が存在しない」状態を作ることである。

## 対象コード

- `src/lib/errorHandler.ts`（`formatUserFriendlyError` / `formatGitError`）
- `src/lib/arrayMove.ts`（`arrayMove`）
- `src/lib/formatRelativeTime.ts`（`formatRelativeTime`）
- `src/lib/markdownUtils.ts`（`isMarkdownFile`）
- #767 に列挙されていた frontend helper のうち、他の分割 ISSUE に属さず未分類で残っているもの

### 現状調査による事実

コード調査で確認した各 helper の現状。本要求はこの事実に基づく。

#### errorHandler.ts — application decision を含むが、本番呼び出し元が存在しない

- `formatUserFriendlyError(error, context)` は raw error 文字列を `toLowerCase` / `includes` で検査し、**error kind を分類して user-facing message と recovery 文言を決定**している（`Cannot read properties of null/undefined`、`network error` / `failed to fetch`、`command ... not found`、`git` 等）。これは責務範囲上「error kind / recovery action の判定」に該当し、frontend に残すべきでない application decision である。
- `formatGitError` はその薄い wrapper。
- ただし `src/` 内で **production からの呼び出し元は存在しない**（参照は `errorHandler.test.ts` のみ。`telemetry.ts` の `errorHandler` は同名の別ローカル変数で無関係）。すなわち application decision は**現状どこにも wire されていない**。

#### arrayMove.ts — 純粋な汎用 UI helper、本番呼び出し元が存在しない

- `arrayMove(arr, from, to)` は範囲チェック付きの汎用配列並べ替え。**domain order（workflow/session/repository の正典順序）を一切含まない**、純粋な component-local reorder helper。
- `src/` 内で **production からの呼び出し元は存在しない**（参照は `arrayMove.test.ts` のみ）。

#### formatRelativeTime.ts — display-only formatting

- `formatRelativeTime(createdAt)` は経過時間を `now` / `Nm` / `Nh` / `Nd` の表示文字列に変換する display-only formatter。
- `src/components/panels/DiffInlineComment.tsx:26` で使用。responsibility 上「display-only formatting」に該当し frontend に残してよい。

#### markdownUtils.ts — UI 表示条件のみ

- `isMarkdownFile(path)` は拡張子（`md` / `mdx` / `markdown`）判定。
- `src/components/panels/ReviewPanel.tsx:302` で **markdown preview トグルを出すかどうかだけの UI 条件**として使用（`isTextDiff && selectedFile ? isMarkdownFile(selectedFile) : false`）。diff read model / comment read model には影響しない。responsibility 上「markdown を表示するかどうかだけの UI condition」に該当し frontend に残してよい。
- ただし test が `src/lib/__tests__/markdownUtils.test.ts` にあり、プロジェクト規約（対象ファイル隣接配置）から外れている。

## スコープ

- 対象 helper（`errorHandler.ts` / `arrayMove.ts` / `formatRelativeTime.ts` / `markdownUtils.ts`）と、#767 残余 helper のうち他 ISSUE に属さないものを 1 つずつ責務分類する。
- **UI-only かつ使用中の helper**: domain/application behavior を持たないことが test と配置（命名・ディレクトリ・コメント等）から判別できる状態にする。
  - `formatRelativeTime.ts`: display-only helper として残す（必要なら既存 UI util へ寄せる）。
  - `markdownUtils.ts`: UI 表示条件 helper として残す。test を隣接配置へ寄せる。
- **未参照（dead）の helper**: 本 ISSUE の対象であり「使っていないと分かっている」ため、本 ISSUE で削除する（分類のために残さない）。
  - `errorHandler.ts`: production 呼び出し元が存在せず、application decision はどこにも wire されていない。Rust 経路を新設せず、ファイル（および test）を削除する。
  - `arrayMove.ts`: production 呼び出し元が存在しない純粋 UI helper。ファイル（および test）を削除する。

## 非スコープ

- backend module relocation そのもの（#1132 等の領域）。
- branch/path derivation（path/branch migration ISSUE の領域）。
- repository status read model（repository status migration ISSUE の領域）。
- agent stream / session state（agent stream/state migration ISSUE の領域）。
- diff / read model（diff read-model migration ISSUE の領域）。
- terminal pane / PTY lifecycle（terminal pane/PTY ISSUE の領域）。
- **本 ISSUE 対象外 helper の dead-code sweep（#878 の領域）**: 本 ISSUE の対象 helper のうち未参照のものは本 ISSUE で削除するが、対象に含まれない frontend 全体の最終 dead-code sweep は #878 が担当する。
- visual redesign / editor 機能の復活。

## 要求事項

- #767 に列挙されていた frontend helper のうち、他の分割 ISSUE に属さないものが**未分類で残っていない**こと。各 helper が「UI-only として残す」「未参照のため削除する」のいずれかに処理されていること。
- 残す UI-only helper（`formatRelativeTime` / `markdownUtils`）が **domain/application behavior を持たない**ことが、test と配置から明確であること。
  - `markdownUtils` の test がプロジェクト規約に従い対象ファイルに隣接配置されていること。
- 未参照 helper（`errorHandler` / `arrayMove`）が、対応する test ともども削除されていること。frontend に wire されていない application decision が残っていないこと。
- 既存の外部から観測可能な振る舞いを壊さないこと。
  - `ReviewPanel` の markdown preview トグル表示条件が従来どおり動作する。
  - `DiffInlineComment` の相対時刻表示が従来どおり動作する。
- 分類・移行後の振る舞いを担保する test が存在すること（frontend test は rendering/interaction を検証し、Rust 側へ移した decision は Rust test で検証する）。
- `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 受け入れ基準の概要

- 対象 4 helper と #767 残余 helper の各々について、「残す（UI-only）」か「削除（未参照）」の処理結果と根拠が spec / 実装に反映されている。
- 残す UI-only helper が application/domain decision を含まないことを test と配置で確認できる。
- `errorHandler` / `arrayMove` が test ともども削除され、frontend に wire されていない application decision が残っていないことを確認できる。
- 既存 UI（markdown preview トグル、相対時刻表示）の挙動が維持されることを test で確認できる。
- 上記 lint / test / build / clippy がすべて green。

## 仮定

- 本 ISSUE は #767 の「分割後の境界整理担当」であり、**新規機能追加ではなく既存 helper の責務分類**である。
- `formatRelativeTime` は display-only であり frontend に残す（responsibility 範囲に明記された判断、確定）。
- `markdownUtils.isMarkdownFile` は現状 `ReviewPanel` の表示トグル条件にのみ使われ、diff/comment read model に影響しないため frontend UI helper として残す（確定）。test を隣接配置へ移す。
- 未参照（dead）の helper は「使っていないと分かっている」ため本 ISSUE で削除する方針（ユーザー合意済み）。これに従い `errorHandler.ts`（application decision を含むが未 wire）と `arrayMove.ts`（純粋 UI helper だが未参照）を test ともども削除する。Rust への migration コードは書かない。
- 「#767 残余 helper のうち他 ISSUE に属さないもの」の網羅は、#767 対象ファイル一覧から他分割 ISSUE 対象を除いた差集合で確定する（実装ステップで棚卸しする）。

## Open Questions

なし（すべて解消済み）。
