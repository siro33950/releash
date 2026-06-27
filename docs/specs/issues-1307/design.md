# Design

clean architecture 移行（milestone [12]）の境界整理 ISSUE #1307 における設計。
`requirements.md` / `behavior.md` を入力とし、frontend helper の責務分類を「残す（UI-only）」「削除（未参照）」のいずれかに確定し、その作業をどう実装・検証するかを定義する。

## 概要

本 ISSUE は新規機能を追加しない。#767 superseded の分割のうち、他分割 ISSUE（branch/path、repository status、agent stream/state、diff read model、terminal/PTY）に属さない残余 frontend helper を 1 つずつ責務分類し、`#878 final dead-code sweep` 実施前に「未分類の残余 helper が存在しない」状態を作る。

確定済みの分類（requirements / behavior の仮定で合意済み）:

| helper | 分類 | 処理 |
| --- | --- | --- |
| `src/lib/errorHandler.ts` | application decision を含むが未 wire（dead） | ファイル + test を削除。Rust 経路は新設しない |
| `src/lib/arrayMove.ts` | 純粋 UI helper だが未参照（dead） | ファイル + test を削除 |
| `src/lib/formatRelativeTime.ts` | display-only formatter（使用中） | frontend に残す |
| `src/lib/markdownUtils.ts` | UI 表示トグル条件（使用中） | frontend に残す。test を隣接配置へ移動 |

加えて、#767 残余 helper の差集合に未分類のものが残らないことを棚卸しで確認する。

## 変更対象

### 削除

- `src/lib/errorHandler.ts`
- `src/lib/errorHandler.test.ts`
- `src/lib/arrayMove.ts`
- `src/lib/arrayMove.test.ts`

### 移動

- `src/lib/__tests__/markdownUtils.test.ts` → `src/lib/markdownUtils.test.ts`（対象ファイル隣接へ）。移動に伴い import パスを `../markdownUtils` から `./markdownUtils` へ修正する。

### 残す（変更なし）

- `src/lib/formatRelativeTime.ts` / `src/lib/formatRelativeTime.test.ts`（既に隣接配置）
- `src/lib/markdownUtils.ts`
- `src/components/panels/DiffInlineComment.tsx`（`formatRelativeTime` の利用元）
- `src/components/panels/ReviewPanel.tsx`（`isMarkdownFile` の利用元）

### 追加（任意・テスト方針に依存）

- 既存 UI 振る舞いの回帰を担保する component test（後述「テスト方針」）。既に十分なら新規追加しない。

## アーキテクチャと責務分割

本 ISSUE は責務「分類」であり、ロジックの移設（Rust 化）は発生しない。これは requirements の調査結果に基づく:

- `errorHandler` は error kind 分類という application decision を含むが、`src/` 内の production 呼び出し元が存在せず（参照は `errorHandler.test.ts` のみ。`telemetry.ts` の `errorHandler` は同名の無関係なローカル変数）、**どこにも wire されていない**。よって Rust への migration コードは書かず、ファイルごと削除する（合意済み方針: 未使用と分かっているものは分類のために残さず削除する）。
- `arrayMove` は domain order を一切持たない純粋な component-local reorder helper で、production 参照が存在しない。削除する。
- `formatRelativeTime` / `isMarkdownFile` は responsibility 範囲上「display-only formatting」「markdown を表示するかどうかだけの UI condition」に該当し、frontend に残してよい（rust-first-logic の許可範囲内）。

結果として、削除後の `src/lib/` には application/domain decision を持つ未 wire helper が残らず、残す helper は名前・配置・test から UI-only と判別できる状態になる。

### Rust 側

本 ISSUE では Rust 側の変更は発生しない。新しい command / usecase / read model を追加しない。

## データモデルまたは型

新規の型・データ構造は導入しない。削除する `ErrorContext` interface（`errorHandler.ts`）は未参照のため、削除に伴い消える。残す helper のシグネチャ（`formatRelativeTime(createdAt: number): string` / `isMarkdownFile(path: string): boolean`）は変更しない。

## 処理フロー

実装は以下の手順で進める。

1. **棚卸し**: #767 対象ファイル一覧から、他分割 ISSUE（branch/path、repository status、agent stream/state、diff read model、terminal/PTY）対象を除いた差集合を確定する。本 ISSUE の対象 4 helper 以外に未分類の残余が存在しないことを確認する。差集合に追加 helper が見つかった場合は、本設計の分類基準（使用中の UI-only は残す / 未参照は削除 / application decision を含むものは Rust へ）に従って処理し、design に追記する。
2. **削除**: `errorHandler.ts` / `errorHandler.test.ts` / `arrayMove.ts` / `arrayMove.test.ts` を削除する。削除後、これらへの import が `src/` 内に残っていないことを grep で確認する（現状ゼロ）。
3. **test 移動**: `src/lib/__tests__/markdownUtils.test.ts` を `src/lib/markdownUtils.test.ts` へ移動し、import パスを修正する。`__tests__` ディレクトリに本 ISSUE 対象外の test（`markdownDiff` / `rehypeSourceLines` / `telemetry`）が残る場合、それらは本 ISSUE のスコープ外として手を付けない。
4. **回帰確認用 test の確認/追加**: behavior.md の維持シナリオ（markdown preview トグル表示条件 / 相対時刻表示）が test で担保されているか確認する。helper 単体 test（`markdownUtils.test.ts` / `formatRelativeTime.test.ts`）は拡張子集合・しきい値の正典挙動をカバーしているため、これを担保とする。利用元 component（`ReviewPanel` / `DiffInlineComment`）の表示条件が helper を介して維持されることは、helper test + 既存 component test の範囲で確認し、不足する場合のみ最小限の component test を追加する。
5. **品質チェック**: `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` をすべて green にする。

## エラー処理

本 ISSUE は helper の分類・削除であり、ランタイムのエラー処理経路を変更しない。

- `errorHandler` の削除は、production に wire されていないため、エラー表示の挙動に影響しない。削除によって参照切れ（コンパイルエラー / lint エラー）が発生しないことを `pnpm build` / `pnpm lint` で担保する。
- `formatRelativeTime` / `isMarkdownFile` の入力前提（数値 timestamp / 文字列 path）は既存実装のまま維持し、エラー処理の追加・変更は行わない。

## テスト方針

- **削除対象**: `errorHandler.test.ts` / `arrayMove.test.ts` は対象ファイルとともに削除する。削除後に test スイートが green であることを確認する（参照切れが残らない）。
- **残す helper（単体 test）**:
  - `markdownUtils.test.ts`: 隣接配置へ移動。`md` / `mdx` / `markdown` と大文字（`MD` / `MDX`）が true、それ以外が false という拡張子集合の正典挙動を検証する（既存内容を維持）。behavior.md の Scenario Outline（`md` / `mdx` / `markdown` / `MD`）をカバーする。
  - `formatRelativeTime.test.ts`: `now`（<1分）/ `Nm`（<1時間）/ `Nh`（<1日）/ `Nd`（以上）のしきい値挙動を検証する（既存内容を維持）。behavior.md のしきい値切り替え（59秒→`now`、60秒→`1m`）に相当する境界は、必要なら境界ケースを補強する。
- **利用元 UI の回帰**: behavior.md の維持シナリオは「helper の出力 = UI の表示」が崩れないことが本質である。helper 単体 test で正典挙動を固定し、`ReviewPanel` の `isMarkdown` 判定式（`isTextDiff && selectedFile ? isMarkdownFile(selectedFile) : false`）と `DiffInlineComment` の `formatRelativeTime(entry.createdAt)` 呼び出しが helper を介する構造を維持することで担保する。既存 component test で不足する表示条件があれば、その条件に限定した最小限の component test を追加する（過剰な Monaco / 統合 test は避ける、テスト方針に準拠）。
- **Rust test**: 本 ISSUE では Rust への decision 移設が無いため、新規 Rust test は不要。既存 `cargo test` が green であることのみ確認する。

## リスクと代替案

- **リスク: 差集合の取りこぼし**。#767 残余 helper の棚卸しが不完全だと、未分類 helper が残り `#878` の前提が崩れる。対策として手順1で #767 ファイル一覧と他分割 ISSUE 対象の差集合を明示的に突き合わせ、対象 4 helper 以外の残余有無を確認する。残余が出た場合は本設計に追記して分類する。
- **リスク: 隠れた動的参照**。`errorHandler` / `arrayMove` が文字列連結・動的 import 等で参照されている可能性。対策として削除前に grep で静的参照ゼロを再確認し、削除後に `pnpm build` / `pnpm test` で参照切れが無いことを担保する（現状調査では静的参照は test のみ）。
- **代替案: errorHandler を Rust へ移設**。application decision を保存するため Rust command/usecase 化する案。ただし production に wire されておらず、移設しても呼び出し元が無く dead のままになる。合意済み方針（未使用は削除）に従い採用しない。将来 user-facing error 整形が必要になった時点で、Rust 側 read model として新規設計する（本 ISSUE のスコープ外）。
- **代替案: markdownUtils を Rust（diff/comment read model）へ寄せる**。`isMarkdownFile` は diff/comment read model に影響せず ReviewPanel の表示トグル条件のみに使われるため、UI condition として frontend に残す方が責務上正しい。Rust 化はしない。

## 仮定

- `formatRelativeTime` は display-only であり frontend に残す（requirements / behavior で確定）。
- `markdownUtils.isMarkdownFile` は ReviewPanel の表示トグル条件にのみ使われ diff/comment read model に影響しないため frontend UI helper として残し、test を隣接配置へ移す（確定）。
- 未参照（dead）の `errorHandler.ts`（未 wire の application decision を含む）と `arrayMove.ts`（純粋 UI helper だが未参照）は test ともども削除し、Rust への migration コードは書かない（合意済み）。
- 相対時刻のしきい値と markdown 拡張子集合は既存実装を正典とし、本 ISSUE では変更しない。
- 「#767 残余 helper のうち他分割 ISSUE に属さないもの」の網羅は、#767 対象ファイル一覧から他分割 ISSUE 対象を除いた差集合で確定し、実装手順1で棚卸しする。現状調査では対象 4 helper 以外に未分類の残余は確認されていない（棚卸しで最終確認する）。
- `src/lib/__tests__/` に残る他 test（`markdownDiff` / `rehypeSourceLines` / `telemetry`）の配置是正は本 ISSUE のスコープ外とする。

## Open Questions

なし。
