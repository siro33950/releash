## 要求

**種別**: リファクタリング（設計境界の固定 / ドキュメント整備）

**ゴール**:
Workflow Engine Evolution の milestone [01]「設計境界の固定」を完了する。具体的には、Releash のワークフローランタイムが将来的に主語として扱うべき未来形モデル（`WorkflowRun` / `NodeDefinition` / `NodeExecution` / `WorkflowCommand` / `WorkflowEvent`）の仕様と、既存モジュールにおける future core / compatibility adapter の境界を、新規設計文書として確定する。runtime behavior は変更しない。

**背景**:
現在のワークフローエンジンは、`Workflow` / `Step` / `WorkflowState` / `worktree_path` を主語とした旧モデルで実装されている。今後 CLI、Skill、structured output、approval 操作を追加していくと、UI 上の主語と engine 上の主語のずれが拡大し、互換性の圧力で設計が崩れるリスクがある。後続マイルストーン（[02] Normalized Workflow 以降）が実装に踏み込む前に、目標モデルと互換性境界をドキュメントとして固定し、北極星（north star）として参照可能にする必要がある。`docs/workflow-engine-evolution-plan.md` に概要は記載済みだが、各モデルのフィールド詳細と既存モジュールの分類はまだ確定していない。

**スコープ（GitHub Issue #1003 / Milestone 57「[01] Workflow Engine Evolution - 設計境界の固定」対応範囲）**:

- 未来形モデルのフィールド・責務をドキュメントとして定義する
  - `WorkflowRun`: workflow template の 1 実行インスタンス
  - `NodeDefinition`: 正規化後の実行単位
  - `NodeExecution`: run 中の node 実行結果
  - `WorkflowCommand`: state を変化させる唯一の入口
  - `WorkflowEvent`: engine が発行する append-only な事実
- 既存モジュールを future core / compatibility adapter のいずれに位置付けるかを明文化する
- 新規設計文書を `docs/workflow-engine-evolution-plan.md` から north star としてリンクする

**スコープ外（後続マイルストーンに委ねる）**:

- Rust の struct / enum としての型コード化（[02] Normalized Workflow 以降）
- `workflow/normalized.rs` / `workflow/run.rs` / `workflow/command.rs` / `workflow/event.rs` の新規モジュール追加
- 既存 `engine.rs` / `commands.rs` / `state.rs` の振る舞い変更
- CLI / API / UI の追加

**制約**:

- runtime behavior は一切変えない（plan doc 281-296 行目の明示的な制約）
- 既存 `Workflow` / `Step` / `WorkflowState` YAML / JSON 互換は維持される前提を文書化する
- ドキュメントは日本語で記述する（既存 plan doc と同じ）
- 出力先は新規 `docs/workflow-engine-model-boundary.md` とし、plan doc からはリンクのみ追加する（plan doc 本文は大きく書き換えない）

**完了条件**（plan doc の `### [01] 設計境界の固定` 完了条件に準拠）:

- 未来形モデル 5 種のフィールド・責務が文書化されている
- 既存モジュール（`schema.rs` / `validation.rs` / `engine.rs` / `state.rs` / `log.rs` / `commands.rs` / `storage.rs` / `contract.rs` / `facet.rs` / `runtime_view.rs` / `diagnostics.rs` / `session_errors.rs` / `builtin*`）が future core か compatibility adapter かを説明できる
- `docs/workflow-engine-evolution-plan.md` から新規設計文書への参照リンクが追加されている
- Rust / フロントエンドの実装コードは変更されない

## 振る舞い定義

```gherkin
Feature: Workflow Engine Evolution の設計境界ドキュメント

  Rule: 未来形モデルの仕様が north star として参照できる
    Scenario: 後続マイルストーンの設計者が未来形モデルの責務を確認する
      Given 設計者が後続マイルストーンに着手しようとしている
      When 設計者が未来形モデルの責務とフィールドを参照する
      Then WorkflowRun / NodeDefinition / NodeExecution / WorkflowCommand / WorkflowEvent それぞれの責務とフィールドが文書から読み取れる

  Rule: 既存モジュールの位置付けが明確になっている
    Scenario: 改修担当者が既存モジュールの扱いを判断する
      Given 改修担当者がワークフローエンジンの既存モジュールに手を入れようとしている
      When 改修担当者が当該モジュールの位置付けを確認する
      Then そのモジュールが future core と compatibility adapter のいずれであるかが文書から読み取れる

  Rule: 設計文書が plan doc から辿れる
    Scenario: 関係者が進化計画から設計境界の詳細を参照する
      Given 関係者が Workflow Engine Evolution の plan doc を起点に情報を辿っている
      When 関係者が設計境界の詳細を求める
      Then plan doc から新規設計文書への参照が存在し、設計境界の詳細に到達できる

  Rule: 設計境界の固定は runtime 振る舞いを変えない
    Scenario: マイルストーン[01]完了時点でランタイムを利用する
      Given Workflow Engine を利用する利用者が既存のワークフロー定義を実行する
      When マイルストーン[01]の作業が完了した状態でランタイムが動作する
      Then ランタイムの振る舞い（既存 Workflow / Step / WorkflowState の挙動・YAML/JSON 互換）は変化していない

  Rule: state を変化させる入口は WorkflowCommand に一本化される
    Scenario: 何らかの主体がワークフローの state を変えようとする
      Given ある主体がワークフローの state を変化させようとしている
      When その主体が engine に対して state 変化を要求する
      Then 要求は WorkflowCommand として表現される経路でしか受理されない

  Rule: engine が発行する事実は append-only な事実列として積み上がる
    Scenario: 何らかのきっかけで engine が事実を発行する
      Given engine が処理を進めて事実を発行する場面に至っている
      When engine が新しい事実を発行する
      Then その事実は WorkflowEvent として既存の事実列に追記され、過去の事実は書き換わらない

  Rule: 実行の単位は WorkflowRun として識別される
    Scenario: 同一の workflow template から複数回の実行が行われる
      Given 1 つの workflow template が複数回実行されている
      When 関係者がある時点の実行を指し示そうとする
      Then その実行は WorkflowRun として他の実行と区別して識別できる

  Rule: 実行単位は正規化された NodeDefinition で表現される
    Scenario: workflow template の中身が engine で扱われる
      Given workflow template がさまざまな記述スタイルで定義されている
      When engine がその template の実行単位を扱う
      Then 各実行単位は NodeDefinition として一貫した正規化済みの表現で扱われる

  Rule: run 中の各実行の結果は NodeExecution として記録される
    Scenario: WorkflowRun の中で個々の実行単位が処理される
      Given WorkflowRun が進行しており、その中で実行単位が処理されている
      When ある実行単位の処理が一区切りつく
      Then その実行の結果は NodeExecution として、所属する WorkflowRun と紐づけて記録される
```

## アーキテクチャ概要

本タスクは runtime 振る舞いを変更しないドキュメント整備であるため、アーキテクチャ概要は「ドキュメント間の責務配置」と「読み手が情報を辿るフロー」を主軸に整理する。

### 責務配置

- `docs/workflow-engine-evolution-plan.md`（既存 / plan doc）:
  - 担当する: マイルストーン全体の進化方針・北極星モデルの概要紹介・各マイルストーンの完了条件・新規設計文書への参照リンクの追加。
  - 担当しない: 未来形モデル各々のフィールド詳細・既存モジュールごとの分類表・将来コードの型シグネチャ。
- `docs/workflow-engine-model-boundary.md`（新規 / boundary doc / north star 本体）:
  - 担当する: 未来形 5 モデル（`WorkflowRun` / `NodeDefinition` / `NodeExecution` / `WorkflowCommand` / `WorkflowEvent`）のフィールドと責務の確定記述・既存 Rust モジュールの future core / compatibility adapter 分類・「設計境界の固定」フェーズで合意した境界条件。
  - 担当しない: Rust の型コード化・モジュール再配置・既存 plan doc の戦略記述の重複転載（参照リンクで賄う）。
- 既存 Rust モジュール群（`src-tauri/src/workflow/`）:
  - 担当する: 現行の runtime 振る舞い（変更なし）。本マイルストーン期間中は実装は据え置く。
  - 担当しない: 未来形モデルの型としての存在・新規 enum/struct 追加。コードへの反映は [02] 以降。

### データ/通信フロー

- 設計境界の参照フロー: plan doc（north star の入口）→ boundary doc（モデル詳細・モジュール分類）→ 既存 Rust モジュール（現状の実装根拠の確認先）。読み手はこの順で辿れば、戦略 → 境界定義 → 現状実装 へ降りられる。
- 既存モジュール位置付けの判定フロー: 改修担当者がモジュールを見る → boundary doc のモジュール分類表で future core / compatibility adapter を確認 → 当該モジュールに対する許容変更の幅を判断（future core は段階的に未来形へ寄せる / adapter は互換のみ）。
- 未来形モデルの参照フロー: 後続マイルストーンの設計者が boundary doc 内の各モデル節（責務・フィールド・他モデルとの関係）を読む → そのまま [02] 以降の型コード化の入力として再利用する。

### 状態Owner

- 未来形モデル 5 種の定義（フィールド・責務・関係）: boundary doc が一次 Owner。plan doc は概要のみで、詳細の正本は boundary doc。
- 既存モジュールの分類（future core / compatibility adapter）: boundary doc が Owner。コード側に分類メタ情報は持たない（注釈追加もしない／runtime に影響しないため）。
- マイルストーン進行状況・完了条件: plan doc が Owner（既存通り）。
- runtime 振る舞いの定義: 既存 Rust モジュール（コード）が Owner（変更なし）。

### 境界

- plan doc と boundary doc の境界: plan doc は「なぜ」「何を目指すか」「いつまでに」を語り、boundary doc は「何が」「どこに」を定義する。両者の間で詳細の重複は持たず、plan doc → boundary doc への一方向リンクで接続する。
- boundary doc とコードの境界: boundary doc は将来形の宣言であり、現行コードへの注釈や型強制を行わない。コードは現行 runtime を変えずに残し、boundary doc とコードの差分は「未対応であること」が前提として明示される。
- スコープ境界: 本マイルストーンの成果物はドキュメント追加・plan doc へのリンク追記のみ。Rust / TS のコード修正・新規モジュール作成・型定義の追加は本タスクの境界の外（[02] 以降）。
- 互換性境界: 既存 `Workflow` / `Step` / `WorkflowState` の YAML / JSON 互換は維持される前提を boundary doc 内で明文化する。boundary doc は互換破壊を要求しない。

### 実装に委ねること

- boundary doc 内の節構成の細部（例: モデルごとに節を分けるか、共通項目をまとめるか）。
- 既存モジュール分類の提示形式（表形式 / 箇条書き / モジュール別節）。
- フィールド一覧の提示粒度（型名抜きの名前と説明で済ますか、暫定の型ヒントを括弧書きするか）— ただし「Rust の struct/enum としての型コード化」はスコープ外なので、確定的なシグネチャは書かない。
- plan doc 側に追記する参照リンクの文言・配置位置（north star 章付近 or [01] マイルストーン節）。
- boundary doc の見出し階層やアンカー名。
- 用語ゆれの最終調整（例: 「実行単位」「ノード」「ステップ」のいずれを主表現にするか）— 振る舞い定義で使われた語彙との整合は取る。

