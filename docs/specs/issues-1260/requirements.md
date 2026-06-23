# Requirements

## Type

WorkspaceList の空状態 (empty state) 表示の追加。

関連: #1260

## 背景と目的

Issue #1260 の要求:

> WorkSpaceList で Worktree 等を開いているが空の時にはないことを明示する

`WorkspaceList` は階層的に展開可能なツリー UI で構成されている。一部の階層には空のときの明示表示があるが、**Worktree を展開したときに配下のノード (Session / Workflow) が 1 件も無い場合は何も描画されない**ため、ユーザーは「中身が空である」のか「読み込みに失敗している / まだ読み込まれていない」のかを区別できない。

現状の空状態表示の有無 (`src/components/workspace/WorkspaceList.tsx`):

| 階層 / 場所 | 空のときの表示 | 状態 |
| --- | --- | --- |
| WorkspaceList トップ (リポジトリ 0 件) | `No Repository` | あり |
| SessionHistory サブメニュー (閉じた Session 0 件) | `No closed sessions` | あり |
| WorkflowHistory サブメニュー (履歴 0 件) | `No workflows` | あり |
| NewWorkflow サブメニュー (定義 0 件) | `No workflows configured` | あり |
| **Worktree 展開時の配下ノード (Session / Workflow 0 件)** | **なし (空白)** | **欠落** |
| **RepoTreeSection 展開時のブランチ一覧 (0 件)** | **なし (空白)** | **欠落 (※後述の仮定参照)** |

目的: Worktree を展開したときに配下のノードが空である場合、その旨を明示するプレースホルダ表示を追加し、「空である」状態を「読み込み中 / エラー」状態と明確に区別できるようにする。

## スコープ

- `WorkspaceList` の **Worktree 展開ノード**配下に、Session / Workflow が 1 件も存在しない場合の空状態プレースホルダ表示を追加する。
- 空状態表示は、既存の **読み込み中 (`treeLoading`)** 表示および **エラー (`treeError`)** 表示と排他的に成立させる (空状態はそのどちらでもない正常完了かつ 0 件のときのみ表示)。
- 既存の空状態表示 (`No Repository` 等) のラベル文言・スタイルと整合する見た目とする。

## 非スコープ

- 既に空状態表示を持つ階層 (WorkspaceList トップ / SessionHistory / WorkflowHistory / NewWorkflow サブメニュー) の文言・挙動の変更。
- Session / Workflow ノードの取得ロジック (`useWorkspaceTreeNodes` 等) の変更。
- 読み込み中 / エラー表示の挙動・文言の変更。
- WorkspaceList のナビゲーション構造・ステータス表示・集約ロジックの変更 (#1259 等で扱う範囲)。
- 中央パネル (Chat UI / Step 画面 / Editor / Terminal) の表示。

## 要求事項

### R1. Worktree 空状態の明示

- Worktree を展開し、配下に Session も Workflow も存在しない (`displayNodes` が空) 場合に、空である旨を示すプレースホルダを表示すること。
- このプレースホルダは、読み込み中 (`treeLoading`) でもエラー (`treeError`) でもない正常完了状態かつ 0 件のときにのみ表示すること。

### R2. 読み込み中・エラーとの非混同

- 読み込み中はプレースホルダを表示せず、既存の読み込み中表示 (スピナー) を維持すること。
- エラー時 (かつノード 0 件) はプレースホルダを表示せず、既存のエラー表示を維持すること。
- これにより「空」「読み込み中」「エラー」の 3 状態が視覚的に区別できること。

### R3. 既存表示との一貫性

- 空状態プレースホルダの文言・配色・インデントは、既存の空状態表示 (`No Repository` 等、`text-muted-foreground` 系の控えめなスタイル) と一貫した見た目とすること。
- 文言は既存の英語短文ラベルに合わせる (仮定 A1 参照)。

### R4. ロジック配置

- 「空であるか」の判定 (ノード件数が 0 か) は表示条件であり、フロントエンドで扱ってよい (rust-first-logic の「受け取ったデータの表示条件」に該当)。新たな Tauri コマンド追加は不要とする。

## 受け入れ基準の概要

- Worktree を展開し配下に Session / Workflow が 1 件も無いとき、空である旨のプレースホルダが表示される。
- 配下に 1 件以上のノードがあるときはプレースホルダが表示されず、従来どおりノード一覧が表示される。
- 読み込み中はスピナーのみが表示され、空プレースホルダは表示されない。
- エラー (かつ 0 件) 時はエラー表示のみで、空プレースホルダは表示されない。
- 上記をカバーする `WorkspaceList.test.tsx` のテストが追加され、`pnpm lint` / `pnpm test` が通る。

## 仮定

- **A1 (文言)**: 空状態の文言は既存ラベルと同系統の英語短文とする。具体的には Worktree 配下の空表示を `No sessions or workflows` とする (確定は A1 のレビューで)。
- **A2 (対象範囲)**: 本変更の主対象は Worktree 展開ノードの空状態とする。`RepoTreeSection` のブランチ一覧は、main worktree が常に 1 件存在するため実運用上 0 件になり得ず、空状態表示の追加対象外とする (Open Question Q1 で確認)。
- **A3 (バックエンド変更なし)**: 既存の `useWorkspaceTreeNodes` が返す `nodes` / `loading` / `error` で判定可能なため、Rust 側の変更は不要とする。

## Open Questions

なし (Q1: 対象範囲は Worktree 展開ノードのみとすることで確定。RepoTreeSection 等は対象外。)
