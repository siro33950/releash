## 要求

**種別**: 改善
**ゴール**: `spec-driven-development` の `plan_spec` フェーズを、詳細な自然言語の実装仕様ではなく、責務境界とフローを示す `Architecture Brief` に置き換える。`requirements` と `behavior` は維持する。
**背景**: 現在の `plan_spec` と plan review 系 step により、実装前に自然言語の詳細設計を完成させようとしてしまい、コードを読んで判断した方がよい細部までPlanに固定される。実装前に必要なのは詳細な処理手順ではなく、責務配置・境界・データフロー・状態Ownerといった「実装の地図」である。

### 方針

新しいPlan段階は以下を目指す:

- 要件と主要な期待挙動は残す
- 実装仕様は詳細化しすぎない
- 実装順は書かない
- 関数内手順や疑似コードは書かない
- 責務配置、境界、データ/通信フロー、状態Ownerを明確にする
- 詳細な実装判断はコード実装時に委ねる
- 実装後レビューを成果物中心にする

### Architecture Brief の内容

Architecture Brief は以下のセクションで構成する:

1. **責務配置**: 各領域について担当すること/担当しないことを整理する
2. **データ/通信フロー**: UI → Tauri command → Rust service → storage → event の流れを短く示す
3. **状態の持ち主**: 各状態のOwnerを明示する
4. **境界**: 各層が守るべき責務の境界を示す
5. **実装に委ねること**: 実装中に判断してよい項目を明示する（helper関数名、内部データ変換、細かいコンポーネント分割、テストケースの具体的配置等）

### 変更対象

- `src-tauri/src/workflow/builtin/spec-driven-development.yml`
- `src-tauri/src/workflow/builtin_facets/instructions/plan-spec.md`
- plan review 系 instruction facets
  - `plan-review-completeness.md`
  - `plan-review-clarity.md`
  - `plan-review-consistency.md`
  - `plan-review-security.md`
- `plan-fix.md`
- 必要に応じて workflow output contract / tests

### 受け入れ条件

- `plan_spec` が Architecture Brief を作る step に置き換わっている
- `requirements` と `behavior` は維持されている
- Architecture Brief に責務配置、境界、データ/通信フロー、状態Ownerが含まれる
- Plan段階で実装順、関数内手順、疑似コード、詳細設計を求めない
- plan review は仕様の完全性ではなく、Architecture Brief として着手可能かを確認する
- 実装後レビューは成果物中心に行われる
- 小〜中規模Issueで、実装前Planが過度に肥大化しない

### 非スコープ

- 要件整理 step の廃止
- 振る舞い定義 step の廃止
- Issue分割
- 複数worktree生成
- Workflow YAMLの自由生成

## 振る舞い定義

```gherkin
Feature: plan_spec を Architecture Brief に置き換える

  spec-driven-development ワークフローにおいて、plan_spec ステップが
  詳細な実装仕様ではなく Architecture Brief を生成するようにする。

  Rule: plan_spec ステップは Architecture Brief を Spec ファイルに記録する

    Scenario: Architecture Brief の生成
      Given 要求と振る舞い定義が Spec ファイルに記録されている
      When plan_spec ステップを実行する
      Then Architecture Brief が Spec ファイルに記録される

    Scenario: Architecture Brief に必須セクションが含まれる
      Given plan_spec ステップが完了している
      Then Architecture Brief に責務配置セクションが含まれる
      And Architecture Brief にデータ/通信フローセクションが含まれる
      And Architecture Brief に状態の持ち主セクションが含まれる
      And Architecture Brief に境界セクションが含まれる
      And Architecture Brief に実装に委ねることセクションが含まれる

  Rule: Architecture Brief は実装の地図であり詳細設計ではない

    Scenario: 実装順を含まない
      Given plan_spec ステップが完了している
      Then Architecture Brief に実装順序の指定が含まれない

    Scenario: 関数内手順や疑似コードを含まない
      Given plan_spec ステップが完了している
      Then Architecture Brief に関数内の処理手順が含まれない
      And Architecture Brief に疑似コードが含まれない

  Rule: requirements と behavior ステップは変更されない

    Scenario: 要求整理ステップの維持
      Given spec-driven-development ワークフローを実行する
      When plan_requirements ステップに到達する
      Then 従来と同じ要求整理が実行される

    Scenario: 振る舞い定義ステップの維持
      Given spec-driven-development ワークフローを実行する
      When plan_behavior ステップに到達する
      Then 従来と同じ振る舞い定義が実行される

  Rule: plan review は Architecture Brief の着手可能性を検証する

    Scenario: 着手可能な Architecture Brief の承認
      Given Architecture Brief に責務配置・境界・フロー・状態Ownerが記載されている
      When plan review を実行する
      Then Architecture Brief が着手可能と判定される

    Scenario: 責務境界が不明確な場合の指摘
      Given Architecture Brief に責務の境界が不明確な箇所がある
      When plan review を実行する
      Then 境界の不明確さが指摘される

    Scenario: 仕様の詳細度を理由に差し戻さない
      Given Architecture Brief が実装の地図として十分な情報を持つ
      And Architecture Brief に詳細な処理手順が記載されていない
      When plan review を実行する
      Then 詳細設計の不足を理由に差し戻されない

  Rule: 実装後レビューは成果物を中心に検証する

    Scenario: 成果物ベースのコードレビュー
      Given 実装が完了している
      When code review を実行する
      Then 受け入れ基準の充足が検証される
      And コードの構造・品質・セキュリティが検証される
      And 詳細設計書との逐一照合は行われない
```

## 実装仕様

**対応方針**: 振る舞い定義を実現するために、spec-driven-development ワークフローの instruction facets（Markdown ファイル群）の内容を書き換え、`plan_spec` ステップの出力を「実装仕様」から「アーキテクチャ概要」に置き換える。ワークフローの構造（ステップ名、遷移、output contract、policy）は変更しない。変更は instruction facet の内容と YAML コメントに限定する。

**対象コンポーネント**:

1. `src-tauri/src/workflow/builtin_facets/instructions/plan-spec.md` (主要変更):
   - 「実装仕様」の整理指示をアーキテクチャ概要の生成指示に全面書き換え
   - Spec ファイルへの記録セクション名を `## 実装仕様` → `## アーキテクチャ概要` に変更
   - テンプレートを5セクション構成（責務配置、データ/通信フロー、状態の持ち主、境界、実装に委ねること）に変更

2. `src-tauri/src/workflow/builtin_facets/instructions/plan-review-completeness.md` (評価基準変更):
   - アーキテクチャ概要の必須5セクションが存在するかの検証を追加
   - 振る舞い定義の検証（要求カバレッジ、エッジケース、エラー系）は維持
   - 詳細設計の欠如をフラグしない方針を明示

3. `src-tauri/src/workflow/builtin_facets/instructions/plan-review-clarity.md` (評価基準変更):
   - 「実装仕様の具体性」→「アーキテクチャ概要の明確性」に変更
   - 「推測なしに実装できるほど具体的か」→「実装着手に十分な地図を提供しているか」に変更
   - 判定基準から「実装仕様が具体的」を削除し、「着手可能な明確さがあるか」に変更
   - フラグしない項目に「詳細な処理手順の不足」を追加

4. `src-tauri/src/workflow/builtin_facets/instructions/plan-review-consistency.md` (参照先変更):
   - 「実装仕様がプロジェクトのレイヤー境界を尊重しているか」→「アーキテクチャ概要の責務配置がプロジェクトのレイヤー境界を尊重しているか」に変更

5. `src-tauri/src/workflow/builtin_facets/instructions/plan-review-security.md` (変更なし):
   - セキュリティ検証は振る舞い定義に対するものであり、アーキテクチャ概要への置き換えの影響を受けない

6. `src-tauri/src/workflow/builtin_facets/instructions/plan-fix.md` (微修正):
   - 修正対象がアーキテクチャ概要を含む Spec であることを文面に反映

7. `src-tauri/src/workflow/builtin_facets/instructions/plan-approval.md` (サマリー変更):
   - サマリーフォーマットの「実装仕様」→「アーキテクチャ概要」に変更
   - 「対応方針、対象コンポーネント」→「責務配置、境界、フロー、状態Owner」に変更

8. `src-tauri/src/workflow/builtin_facets/instructions/implement.md` (参照先変更):
   - 入力セクション `## 実装仕様` → `## アーキテクチャ概要` に変更
   - 「実装仕様の批判的評価」→「アーキテクチャ概要の活用」に変更（地図として参照）
   - 「実装仕様の実装順序に従い」→ 実装順序の指定を削除（アーキテクチャ概要には実装順がない）

9. `src-tauri/src/workflow/builtin_facets/policies/plan-review.md` (評価方針変更):
   - アーキテクチャ概要として着手可能かを評価する方針に変更
   - 詳細設計の不足を指摘しない方針を追加

10. `src-tauri/src/workflow/builtin/spec-driven-development.yml` (コメント更新のみ):
    - ステップ 3〜7 のコメントを「実装仕様」→「アーキテクチャ概要」に変更
    - ステップ名（`plan_spec` 等）、遷移（`pass_output_from`）、output contract は変更しない

**変更しないもの**:
- `builtin.rs`: ファイル名・キー名を変更しないため、`include_str!` パスやファセット登録は不変
- Rust コード全般（`schema.rs`, `engine.rs`, `validation.rs`, `contract.rs` 等）: ワークフロー構造を変更しないため不変
- Output contracts（`spec-file-path.md`, `review-verdict.md`）: 出力フォーマットは不変
- `plan-requirements.md`, `plan-behavior.md`: 要求整理と振る舞い定義は維持
- コードレビュー系 instruction（`review-acceptance.md` 等）: Spec ファイルの `## 実装仕様` を直接参照する記述がないため影響なし（`implement.md` 側でアーキテクチャ概要参照に変更済み）

**影響するテスト**:
- `builtin.rs` テスト: ファセットの追加・削除がないため、既存テスト（ファセット数カウント `instructions.len() == 19` 等）はそのままパスする
- YAML パース/バリデーション テスト: ステップ名・構造が不変のため影響なし
- フロントエンド テスト: ワークフロー UI は instruction 内容に依存しないため影響なし
- CI 確認: `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` で既存テストの通過を確認する
