# issues-927: ステップカードのUI改善

## 要求

**種別**: 改善

**ゴール**: ワークフロー実行画面のステップカードから冗長・特殊な要素を取り除き、情報の重複や認知負荷を減らした表示にする。具体的には次の4点が達成されている状態とする。

1. 「#1」「#2」のようなフロー内順序を示すバッジが表示されていない（フローのUIを見れば順序は自明）
2. 「Result: ...」のテキストと、同じ値を示すVerdictバッジの二重表示が解消されている
3. 「run 1」「run 2」のような同一ステップの実行回数バッジが表示されていない
4. Structured Output の値が `spec_file_path` を持つときだけ表示される専用リンクボタンが表示されていない（Output内容に応じて出し分ける特殊UIをやめる）

**背景**: 現状のステップカードには、フローの並びを見れば自明な順序バッジ、同じ情報を二重に伝えるResultテキスト＋バッジ、ニュアンスが伝わりづらい "run N" 表記、Outputが特定形状（`spec_file_path` を含む）のときだけ現れる特殊なリンクUIなど、ノイズや一貫性のない要素が混在している。これらを整理して、ステップカードを「必要十分な情報のみが載っている、シンプルで一貫したカード」にしたい。

**影響範囲**:
- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowTrace.tsx` の通常ステップ行（StepRow相当）と並列ブロック行（ParallelBlockRow）の両方
- `StructuredOutputToggle` の `spec_file_path` 専用リンク表示
- 関連テスト（`WorkflowTrace.test.tsx` など）

## 振る舞い定義

```gherkin
Feature: ステップカードのUI改善

  Rule: フロー内順序の表示
    ステップの並び順はワークフローのUIから自明であり、カード上に順序バッジを重ねて表示しない。

    Scenario: 複数ステップから成るワークフローを表示する
      Given ワークフローに2つ以上のステップが定義されている
      And いずれかのステップが実行済みまたは実行中である
      When ワークフロー実行画面のステップカードが描画される
      Then 「#1」「#2」のようなフロー内順序を示すバッジはどのステップカードにも表示されない

    Scenario: 1ステップのみのワークフローを表示する
      Given ワークフローに1つだけステップが定義されている
      And そのステップが実行済みまたは実行中である
      When ワークフロー実行画面のステップカードが描画される
      Then 「#1」というフロー内順序を示すバッジはステップカードに表示されない

  Rule: 実行結果(Result)の表示
    完了ステップの結果は1か所（Verdictバッジ）でのみ伝え、同じ値を別のテキストとして重複して表示しない。完了ステップごとに Verdict バッジは最大1つに正規化する。

    Scenario: 結果を持つ完了ステップを表示する
      Given ステップが完了状態である
      And そのステップが結果文字列（例: "LGTM" や "completed"）を持つ
      When ステップカードが描画される
      Then 結果は Verdict バッジとしてのみ表示される
      And 同じ値を含む「Result: ...」というテキスト行は表示されない

    Scenario: 結果文字列を持たない完了ステップを表示する
      Given ステップが完了状態である
      And そのステップが結果文字列を持たない
      When ステップカードが描画される
      Then 「Result: completed」のような Result テキスト行は表示されない

    Scenario: 並列ブロック全体の結果を表示する
      Given 並列ステップブロックが完了状態である
      And ブロック全体の結果文字列を持つ
      When 並列ブロック行が描画される
      Then 「Result: ...」というテキスト表記は表示されない
      And ブロック全体の結果を示す Verdict バッジも表示されない（本要求では並列ブロック行の Result テキスト行を除去するのみで、ブロックレベルの VerdictBadge は新たに追加しない）

    Scenario: collect/reduce で同一結果が重複し得る完了ステップを表示する
      Given collect ステップが完了状態である
      And ステップ自身の結果文字列と reduce 結果が同値である
      When ステップカードが描画される
      Then 結果を示す Verdict バッジは1つだけ表示される
      And 同一結果の Verdict バッジが2つ並んで表示されることはない

  Rule: 同一ステップの実行回数(run)の表示
    同一ステップの何回目の実行かを示すバッジは表示しない。

    Scenario: 同一ステップが繰り返し実行される
      Given あるステップが同一ワークフロー内で2回以上実行された
      When それぞれの実行に対応するステップカードが描画される
      Then 「run 1」「run 2」のような実行回数を示すバッジはどのカードにも表示されない

    Scenario: 通常ステップが初回実行中である
      Given 通常ステップがワークフロー内で初めて実行中である
      When そのステップのカードが描画される
      Then 「run 1」のような実行回数を示すバッジは表示されない

    Scenario: 通常ステップが初回完了である
      Given 通常ステップがワークフロー内で1回だけ実行され完了している
      When そのステップのカードが描画される
      Then 「run 1」のような実行回数を示すバッジは表示されない

    Scenario: 並列ブロック行を表示する
      Given 並列ステップブロックが描画対象である
      When 並列ブロック行が描画される
      Then 「run N」バッジは表示されない

  Rule: Structured Output の表示
    Structured Output は内容の形状に依存しない一貫したUI（トグル）でのみ閲覧でき、特定フィールドに対する専用UIは持たない。

    Scenario: spec_file_path を含む Structured Output を表示する
      Given ステップの Structured Output が `spec_file_path` フィールドを持つ
      When ステップカードが描画される
      Then `spec_file_path` の値をリンクとして直接表示する専用ボタンは表示されない
      And Structured Output は通常のトグル展開によってのみ閲覧できる

    Scenario: spec_file_path を含まない Structured Output を表示する
      Given ステップの Structured Output が `spec_file_path` フィールドを持たない
      When ステップカードが描画される
      Then Structured Output は通常のトグル展開によって閲覧できる
      And `spec_file_path` の有無に関わらず、専用リンクボタンは表示されず、同じ Structured Output トグルボタンと展開後の JSON pre のみで閲覧できる
```

## アーキテクチャ概要

本変更はプレゼンテーション層に閉じたUI整理であり、通信プロトコル・Rust側ロジック・状態管理には手を入れない。ただしフロントエンド側では「描画しなくなったことで不要になった実装は全て削除する」方針とする（描画停止だけでなく、付随するデータ参照・props・helper・import・テストを残骸として残さない）。

### 責務配置
- `src/components/panels/AgentChatPanel/WorkflowPanel/WorkflowTrace.tsx`: ステップカードの見た目を構成する責務。通常ステップ行（`TraceItemRow` 内のヘッダ）、`ParallelBlockRow` のヘッダ・サマリ、`TraceItemSummary` のResult表示、`StructuredOutputToggle` の `spec_file_path` 専用リンクを削除し、削除に伴って不要になった内部変数・helper・props・importも併せて削除する責務 / バックエンドからの取得・整形・状態遷移は担当しない
- `WorkflowTrace.tsx` 内のデータ変換層（`buildTraceItems`）: `stepHistory` から `TraceItem[]` を組み立てる責務は維持。`occurrence` は `run N` 表示用途からは外す。ただし React key 安定化（`traceItemKey`）や現在ステップの出現回数計算に必要な値、または同等の内部識別子は維持する。表示削除によって完全に未使用となった派生フィールドのみ削除し、`TraceItem` の型からも未使用フィールドのみを除去する
- バックエンド（Rust / WebSocket protocol / `workflow` 関連型）: 何も担当しない（変更なし） / 表示形式の整理に巻き込まれない
- 型定義 `src/types/workflow.ts`: バックエンドと共有する型は変更しない / フロントローカルで参照される派生型が未使用になる場合に限り、フロント側で削除する

### データ/通信フロー
- ステップ完了の表示: 既存と同じ「WorkflowState → `buildTraceItems` → `TraceItemRow` / `ParallelBlockRow`」。本変更では末端のJSX出力から `#N` バッジ・`run N` バッジ・`Result: ...` テキスト・`spec_file_path` 専用リンクの描画分岐を除去する。中間データのうち、run バッジ表示にのみ使われる派生値（`occurrence` の `run N` 描画用途）は削除する。ただし `seenCounts` のうち以下の用途で参照されている内部値は表示しなくなっても維持する: (a) `currentAlreadyCompleted` 判定（現在ステップが履歴に既出かの判定）、(b) 現在ステップを TraceItem として追加する際の出現回数計算、(c) React key の安定性確保
- Structured Output の閲覧: 「`StructuredOutputToggle` の展開ボタン → JSON pre 表示」のフローのみに一本化。`spec_file_path` を検出する分岐と `onFileClick` 経由のリンクボタンを除去する
- `onFileClick` プロパティの去就: `StructuredOutputToggle` が `spec_file_path` 専用リンク以外に使っていなければ、`StructuredOutputToggle` および呼び出し元（`WorkflowTrace` / `ParallelBlockRow` / `ParallelChildRow`）の `onFileClick` 受け渡しは全削除する。`WorkflowTraceProps` の `onFileClick`、さらに上位の呼び出し元から渡されている経路も実装時に辿って削除する

### 状態Owner
- ステップ履歴・結果文字列・Structured Output: Rustバックエンド（`workflow` ステート）が Owner。フロントは props 経由で受け取るのみ（変更なし）
- `StructuredOutputToggle` の展開状態 (`expanded`): `StructuredOutputToggle` 内部の `useState`（変更なし）
- 表示の出し分けロジック: `WorkflowTrace.tsx` 内の各サブコンポーネントが Owner。本変更で「出し分け自体を廃止する」とともに、出し分けに使っていたローカル状態・派生値もOwner側から削除する

### 境界
- 振る舞いの実現は `WorkflowTrace.tsx`（および `WorkflowTrace.test.tsx`、`onFileClick` を渡している上位呼び出し元）の編集のみで完結させる。Tauriコマンド・WebSocketプロトコル・Rust側ロジック・バックエンド共有型には触れない
- 「描画停止のみ」の中途半端な状態を残さない: 表示削除に伴って参照されなくなった変数・関数・props・import・型フィールド・テストヘルパーは全て削除する
- Verdictバッジ（`VerdictBadge`）のスタイル・対象値は変更しない。Resultテキスト行を除去し、完了ステップごとの結果表示は最大1つの Verdict バッジに正規化する。collect/reduce 由来で同一結果（`item.entry.result` と `reduceResult` が同値）が重複し得る場合も、Verdict バッジが2つ並ばないよう重複表示を抑止する
- 並列子ステップ行（`ParallelChildRow`）内の `verdict` バッジと「verdictが無い場合の child.result バッジ」表示は本要求のスコープ外であり、変更しない
- `EventTrace` および `CurrentAction` は本要求のスコープ外であり、変更しない

### 実装に委ねること
- 削除に伴って未使用になる変数・分岐・import・型フィールド（例: `occurrence` の run バッジ表示専用用途、`spec_file_path` 検出ロジック、`onFileClick` プロパティ連鎖、`TraceItem` 内の未使用フィールド）の具体的な削除手順。なお `seenCounts` は run バッジ表示専用の派生値（`occurrence` の表示用途）のみを削除対象とし、`currentAlreadyCompleted` 判定・現在ステップの出現回数計算・React key 安定性確保に必要な内部カウンタは維持する
- `StructuredOutputToggle` のレイアウト微調整（`spec_file_path` ボタン除去後の余白・縦並び）
- `TraceItemSummary` 完了ブロックの「Result行」除去後にトークン数・Viewボタンをどの行構造で残すかの細かなレイアウト
- `WorkflowTrace.test.tsx` の既存テストのうち、削除対象UIを検証していたテストの扱い（削除 or 反転：「表示されない」の検証へ書き換え）と新規テストケースの具体的な書き方
- 並列ブロック行サマリの「`completedCount`/`totalCount` completed」「tokens」表示の維持・整列方法
- `onFileClick` を `WorkflowTrace` に渡している呼び出し元の追跡範囲と、上位プロパティ削除のチェイン

