## 要求

**種別**: バグ修正
**ゴール**: ワークフロー画面の全カードがパネル幅内に収まるように修正する
**背景**: 現在、ワークフロー画面のトレース表示でステップカードがパネルの幅を超えてはみ出して表示され、UIが崩れている（Issue #906 スクリーンショット参照）。TraceItemRowで確認済みだが、他のカード（ParallelBlockRow、CurrentAction等）でも同様の問題が発生していないか確認し、必要があれば併せて修正する

### 現在の挙動

- ワークフロートレースのステップカード（TraceItemRow）全体がパネルの幅を超えてはみ出して表示される
- 他のカード（ParallelBlockRow、CurrentAction）でも同様の問題が発生している可能性がある

### 期待する挙動

- ワークフロー画面の全カードがパネル幅に収まり、内部コンテンツは適切にtruncate・折り返しされる

### 対象コンポーネント

- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowTrace.tsx` — TraceItemRow（確認済み）、ParallelBlockRow、CurrentAction（要確認）
- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowPanel.tsx` — その他パネル内カード（要確認）

## 振る舞い定義

```gherkin
Feature: ワークフロートレースのカードレイアウト

  ワークフロー画面のトレース表示において、全てのカードがパネル幅内に収まる

  Rule: トレースカードはパネル幅内に収まる

    Scenario: 完了ステップカードがパネル幅内に表示される
      Given ワークフローが実行され完了したステップのトレースが存在する
      When トレース画面が表示される
      Then 完了ステップカードがパネル幅内に収まっている

    Scenario: 並列ブロックカードがパネル幅内に表示される
      Given ワークフローが実行され並列ステップのトレースが存在する
      When トレース画面が表示される
      Then 並列ブロックカードがパネル幅内に収まっている

    Scenario: 実行中アクションカードがパネル幅内に表示される
      Given ワークフローが実行中である
      When トレース画面が表示される
      Then 実行中アクションカードがパネル幅内に収まっている

  Rule: 長いコンテンツは適切に処理される

    Scenario: パネル幅を超えるステップ名が省略表示される
      Given ステップ名がパネル幅を超える長さである
      When トレースカードが表示される
      Then ステップ名がtruncateされ省略記号付きで表示される

    Scenario: パネル幅を超える出力テキストが折り返される
      Given ステップの出力テキストがパネル幅を超える長さである
      When 出力テキストが展開表示される
      Then テキストがパネル幅内で折り返されて表示される

  Rule: パネルリサイズ時もレイアウトが維持される

    Scenario: パネルリサイズ後もカードが収まる
      Given トレース画面にカードが表示されている
      When パネル幅がリサイズされる
      Then 全てのカードがリサイズ後のパネル幅内に収まっている
```

## 実装仕様

**対応方針**: CSS Grid/Flexbox のオーバーフロー制約不足によるカードはみ出しを、`min-w-0`/`overflow-hidden` の追加とバッジ行の `flex-wrap` 対応で修正する。ロジック変更なし、CSSクラスのみの修正。

### 根本原因

CSS Grid の `1fr` カラムはコンテンツの固有サイズ（intrinsic size）が大きい場合にカラム幅を超えて描画される。Flexbox の子要素も `min-width: auto`（デフォルト）のため、テキストやバッジが縮小されずコンテナ幅を超える。

### 修正箇所

**対象コンポーネント**:

- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowTrace.tsx`:
  - **TraceItemRow**: Grid `1fr` カラムのカードdivに `overflow-hidden` を追加。バッジ行（stepMode, #N, run N）に `flex-wrap` と `min-w-0` を追加し、ステップ名の `truncate` が正しく機能するようにする
  - **ParallelBlockRow**: 同様に Grid `1fr` カラムのカードdivに `overflow-hidden` を追加。バッジ行に `flex-wrap` と `min-w-0` を追加
  - **ParallelChildRow**: 子ステップ行に `min-w-0` を追加し、長いステップ名の truncate を有効化
  - **CurrentAction**: カード内の flex コンテナに `overflow-hidden` を追加
  - **StructuredOutputToggle**: `<pre>` タグは既に `whitespace-pre-wrap break-words overflow-auto` を持つため変更不要
  - **EventTrace**: イベントログ行は短いテキストのため変更不要

- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowPanel.tsx`: 変更なし（レイアウト構造自体は正しく、問題はトレースカード内のオーバーフロー制約）

### 修正パターン（共通）

1. Grid `1fr` カラムの直接の子に `overflow-hidden` を追加 → コンテンツが `1fr` 幅を超えないようにする
2. テキスト+バッジの flex 行に `min-w-0 flex-wrap` を追加 → バッジがはみ出す場合に折り返し
3. truncate 対象の親に `min-w-0` を確保 → `text-overflow: ellipsis` が正しく機能する条件

**影響するテスト**:

- `WorkflowTrace.test.tsx`（既存テストがある場合）: CSSクラスの変更のみのため、スナップショットテストがある場合は更新が必要。ロジックテストへの影響なし
- 目視確認: 修正後にワークフロー画面でトレースカードがパネル幅内に収まること、長いステップ名がtruncateされること、リサイズ時にレイアウトが維持されることを確認
