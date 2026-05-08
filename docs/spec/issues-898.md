## 要求

**種別**: 新機能

**ゴール**: Spec駆動開発用の built-in Workflow (`spec-driven-development`) を追加する。task入力・既存Specファイルなしでも起動可能。要求整理→振る舞い定義→実装仕様ドラフト→6観点並列レビュー→修正→承認→実装→品質チェック→6観点並列コードレビュー→修正→品質チェック→承認の14ステップを自動遷移する。

**背景**: 現在、Spec駆動の開発プロセスはClaude Codeスキル（/plan-requirements → /plan-behavior → /plan-spec → /implement → /review）として手動で順次実行している。これらのスキルの内容をinstruction facetとしてワークフローエンジンに組み込むことで、ステップ間の受け渡し・レビューの並列実行・修正ループの自動遷移を実現する。

### 基本方針

- `prompt.template` 方式ではなく facet-based prompt に統一
- task入力は任意（ありなら requirements step の初期コンテキスト、なしなら対話で収集）
- 既存Specファイルは必須入力ではない
- requirements step 完了時点でSpecファイルを必ず作成または更新する（失敗時はworkflow停止）
- Specファイルをworkflow内 canonical Spec として扱う
- Plan / spec fix step は同じSpecファイルを更新する
- 後続stepはSpec本文の丸ごと受け渡しではなく、`SPEC_FILE_PATH: ...` を受け取って必要時にファイルを読み込む
- 初版では `SPEC_FILE_PATH` を step output に含め、`pass_previous_response` / `pass_output_from` 経由で後続stepに渡す
- review は6観点（acceptance/structure/quality/test/security/architecture）で並列実行
- review verdict は `LGTM` / `NEEDS_FIX` に統一、集約は `collect.reduce: any_needs_fix`
- 1つでも `NEEDS_FIX` があれば fix step へ戻す、すべて `LGTM` なら approval step へ進む
- Reject時は #897 の Reject comment を次の fix step が `pass_previous_response` で受け取る

### Specファイルの扱い

**作成タイミング:**
- `requirements` step が対話で要求を確定した時点でSpecファイルを作成または更新
- `behavior` / `spec_draft` / `spec_fix` は同じSpecファイルを更新
- `spec_approval` 以降のstepは承認済みSpecファイルを正本として扱う

**パス決定（優先順位）:**
1. 既存Specファイルが明示された場合: そのパス
2. issue番号が分かる場合: `docs/spec/issues-XXX.md`
3. PJT番号が分かる場合: `docs/spec/PJT-XXXX.md`
4. どちらも分からない場合: `docs/spec/workflow-{execution_id}.md`

**step output contract:**
Specファイルを作成・更新するstepは、最後に `SPEC_FILE_PATH: docs/spec/issues-XXX.md` を必ず出力する。

### Workflow構成

1. 要求整理（interactive、Spec作成）
2. 振る舞い定義（interactive、Spec更新）
3. 実装仕様ドラフト（interactive、Spec更新）
4. 実装仕様レビュー 6観点（parallel auto）
5. 実装仕様レビュー集約（collect/reduce）
6. 実装仕様修正（auto、Spec更新）
7. 実装仕様承認（approval）
8. 実装（auto）
9. 品質チェック（auto）
10. コードレビュー 6観点（parallel auto）
11. コードレビュー集約（collect/reduce）
12. 修正（auto）
13. 修正後品質チェック（auto）
14. 実装結果承認（approval）

### Built-in facets

**personas / policies**: 既存を利用（planner/coder/reviewer、coding/review）

**instructions 追加（21個）:**

| instruction facet | 元ネタスキル | 用途 |
|---|---|---|
| `plan-requirements-workflow` | plan-requirements SKILL.md | 対話で要求整理→Spec作成 |
| `plan-behavior-workflow` | plan-behavior SKILL.md | Gherkin振る舞い定義→Spec更新 |
| `plan-spec-workflow` | plan-spec SKILL.md | 実装仕様整理→Spec更新 |
| `review-acceptance-spec` | review/references/acceptance.md | Spec要求充足レビュー |
| `review-structure-spec` | review/references/structure.md | Spec構造レビュー |
| `review-quality-spec` | review/references/quality.md | Spec品質レビュー |
| `review-test-spec` | review/references/test.md | Specテスト設計レビュー |
| `review-security-spec` | review/references/security.md | Specセキュリティレビュー |
| `review-architecture-spec` | review/references/architecture.md | Specアーキテクチャレビュー |
| `spec-fix-workflow` | fix.md + plan系 | レビュー指摘に基づくSpec修正 |
| `spec-approval-workflow` | report.md相当 | Spec承認画面の指示 |
| `implement-workflow` | implement SKILL.md | Spec読み込み→実装 |
| `quality-check-workflow` | implement SKILL.md §5 品質ゲート | lint/test/build 実行・失敗時修正 |
| `review-acceptance-code` | review/references/acceptance.md | コード要求充足レビュー |
| `review-structure-code` | review/references/structure.md | コード構造レビュー |
| `review-quality-code` | review/references/quality.md | コード品質レビュー |
| `review-test-code` | review/references/test.md | コードテストレビュー |
| `review-security-code` | review/references/security.md | コードセキュリティレビュー |
| `review-architecture-code` | review/references/architecture.md | コードアーキテクチャレビュー |
| `implement-fix-workflow` | fix.md + implement系 | コードレビュー指摘に基づく修正 |
| `implementation-approval-workflow` | report.md相当 | 実装結果承認画面の指示 |

**output_contracts**: 既存の `spec-file-path`, `review-verdict` を利用

**facet変換の方針:**
- スキルのClaude Code固有操作（AskUserQuestion、Read/Write/Grep等のツール指示、ブランチ名取得等）は除去し、タスクの本質的な指示のみを抽出する
- スキルの確認項目・判定基準・フォーマット定義はそのまま活用する
- review系はスキルの共通レビューポリシー（ファクトチェック義務、スコープ判定、共通FAIL基準）をpolicy facet (`review`) で提供し、各観点固有の検証手順をinstruction facetに記述する
- spec review とcode review は同じ観点だが対象が異なる（Specドキュメント vs コード変更差分）ため、別のinstruction facetとして定義する
- テンプレート変数 `{{project_name}}`, `{{task}}` は既存の仕組みをそのまま利用する

### 依存関係

- #896: step output / collect-reduce 基盤
- #897: approval Reject comment を fix step に渡す
- #862: review 観点ごとの parallel child step 実行
- #909: SPEC_FILE_PATH / review verdict / fix result を structured workflow output として確定

### 非スコープ

- 既存Claude Codeスキル本文そのものの大規模再設計
- GitHub PRレビューコメント投稿連携
- 任意スクリプトreducer
- workflow artifact / variable 基盤の実装

## 振る舞い定義

```gherkin
Feature: Spec駆動開発ビルトインワークフロー
  既存のClaude Codeスキル（plan-requirements/plan-behavior/plan-spec/implement/review）の
  知見をinstruction facetに変換し、14ステップのワークフローとして自動遷移させる。

  Rule: ビルトイン初期化
    Scenario: ビルトインfacetsが起動時に展開される
      Given Releashが起動されていない
      When Releashが起動する
      Then 21個のinstruction facetがfacetsディレクトリに展開される
      And "spec-driven-development" ワークフローYAMLがworkflowsディレクトリに展開される

    Scenario: 既存のビルトインfacetsは上書きされない
      Given 21個のinstruction facetが既に展開済みで内容が同一である
      When Releashが起動する
      Then facetファイルの更新日時は変更されない

    Scenario: 変更されたビルトインfacetsは上書きされる
      Given ビルトインfacetが展開済みだが内容がビルトイン定義と異なる
      When Releashが起動する
      Then facetファイルが最新のビルトイン定義で上書きされる

  Rule: ワークフロー起動
    Scenario: task入力ありで起動する
      Given "spec-driven-development" ワークフローが利用可能である
      When ユーザーがtask入力ありでワークフローを開始する
      Then requirementsステップがtaskを初期コンテキストとして開始される

    Scenario: task入力なしで起動する
      Given "spec-driven-development" ワークフローが利用可能である
      When ユーザーがtask入力なしでワークフローを開始する
      Then requirementsステップが対話モードで開始される

  Rule: Specファイル作成と受け渡し
    Scenario: requirementsステップがSpecファイルを作成する
      Given requirementsステップが実行中である
      When ステップが完了する
      Then Specファイルが作成される
      And ステップ出力に "SPEC_FILE_PATH: {パス}" が含まれる

    Scenario: behavior/spec_draftステップがSpecファイルを更新する
      Given 前ステップでSpecファイルが作成済みである
      When behaviorまたはspec_draftステップが完了する
      Then 同じSpecファイルが更新される
      And ステップ出力に "SPEC_FILE_PATH: {パス}" が含まれる

    Scenario: 後続ステップがSPEC_FILE_PATHで参照する
      Given spec_draftステップが "SPEC_FILE_PATH: docs/spec/issues-123.md" を出力済みである
      When 後続ステップが開始される
      Then ステップのプロンプトにstep_outputとしてSPEC_FILE_PATHが注入される

  Rule: Specファイルパス決定
    Scenario: issue番号からパスを決定する
      Given ワークフローのコンテキストにissue番号898が含まれる
      When Specファイルのパスを決定する
      Then パスは "docs/spec/issues-898.md" となる

  Rule: Specレビュー並列実行
    Scenario: 6観点が並列で実行される
      Given spec_draftステップが完了している
      When spec_review_parallelステップが開始される
      Then acceptance/structure/quality/test/security/architectureの6つのレビューが並列で開始される
      And 各レビューにreviewerペルソナとreviewポリシーが合成される

    Scenario: 全観点LGTMでspec_approvalへ進む
      Given 6つのSpecレビューが全て完了している
      And 全てのレビュー結果が "LGTM" である
      When 集約ステップが評価される
      Then ワークフローはspec_approvalステップへ遷移する

    Scenario: 1つでもNEEDS_FIXでspec_fixへ戻る
      Given 6つのSpecレビューが全て完了している
      And 少なくとも1つのレビュー結果が "NEEDS_FIX" である
      When 集約ステップが評価される
      Then ワークフローはspec_fixステップへ遷移する

  Rule: Spec修正ループ
    Scenario: spec_fixがレビュー指摘を受けてSpecを修正する
      Given spec_fixステップが開始されている
      And レビュー集約結果がpass_output_fromで渡されている
      When spec_fixステップが完了する
      Then Specファイルが更新される
      And ステップ出力に "SPEC_FILE_PATH: {パス}" が含まれる
      And ワークフローはspec_review_parallelへ再遷移する

    Scenario: spec修正ループがcycle_guardで停止する
      Given spec_fixステップが3回実行されている
      When spec_fixステップの完了後にspec_review_parallelへ再遷移しようとする
      Then ワークフローは失敗として停止する

  Rule: Spec承認
    Scenario: Spec承認でApproveする
      Given spec_approvalステップが待機中である
      When ユーザーがApproveを選択する
      Then ワークフローはimplementステップへ遷移する

    Scenario: Spec承認でRejectする
      Given spec_approvalステップが待機中である
      When ユーザーがRejectコメント付きで拒否する
      Then ワークフローはspec_fixステップへ遷移する
      And Rejectコメントがspec_fixステップにpass_previous_responseで渡される

  Rule: 実装と品質チェック
    Scenario: implementステップがSpecに基づいて実装する
      Given spec_approvalで承認されたSpecが存在する
      When implementステップが実行される
      Then SPEC_FILE_PATHのSpecファイルを参照して実装が行われる

    Scenario: 実装後に品質チェックが実行される
      Given implementステップが完了している
      When quality_checkステップが開始される
      Then lint/test/buildが実行される

  Rule: コードレビュー並列実行
    Scenario: コードレビュー6観点が並列で実行される
      Given quality_checkステップが完了している
      When code_review_parallelステップが開始される
      Then acceptance/structure/quality/test/security/architectureの6つのコードレビューが並列で開始される
      And 各レビューにSpec情報と実装結果がpass_output_fromで渡される

    Scenario: 全観点LGTMでimplementation_approvalへ進む
      Given 6つのコードレビューが全て完了している
      And 全てのレビュー結果が "LGTM" である
      When 集約ステップが評価される
      Then ワークフローはimplementation_approvalステップへ遷移する

    Scenario: 1つでもNEEDS_FIXでfixへ戻る
      Given 6つのコードレビューが全て完了している
      And 少なくとも1つのレビュー結果が "NEEDS_FIX" である
      When 集約ステップが評価される
      Then ワークフローはfixステップへ遷移する

  Rule: コード修正ループ
    Scenario: fixステップがレビュー指摘に基づいて修正する
      Given fixステップが開始されている
      And コードレビュー集約結果がpass_output_fromで渡されている
      When fixステップが完了する
      Then ワークフローはfix_quality_checkへ遷移する

    Scenario: fix後の品質チェックが実行される
      Given fixステップが完了している
      When fix_quality_checkステップが開始される
      Then lint/test/buildが実行される
      And 完了後にcode_review_parallelへ再遷移する

    Scenario: コード修正ループがcycle_guardで停止する
      Given fixステップが5回実行されている
      When fixステップの完了後にcode_review_parallelへ再遷移しようとする
      Then ワークフローは失敗として停止する

  Rule: 実装結果承認
    Scenario: 実装結果をApproveする
      Given implementation_approvalステップが待機中である
      When ユーザーがApproveを選択する
      Then ワークフローは完了する

    Scenario: 実装結果をRejectする
      Given implementation_approvalステップが待機中である
      When ユーザーがRejectコメント付きで拒否する
      Then ワークフローはfixステップへ遷移する
      And Rejectコメントがfixステップにpass_previous_responseで渡される

  Rule: facet合成
    Scenario: 各ステップに適切なfacetが合成される
      Given ワークフローのステップが開始される
      When ステップのプロンプトが構築される
      Then ステップ定義のpersona/policy/instruction/output_contractが合成される
      And personaはsystem_promptとして設定される
      And instruction/policy/output_contractはuser_messageとして結合される
```

## 実装仕様

**対応方針**: 既存のビルトイン初期化機構（`builtin.rs` の `init_builtin_facets` / `init_builtin_workflows`）に21個のinstruction facet MDファイル + 1個のワークフローYAMLファイルを追加する。エンジン側の変更は不要（既存の並列実行・collect/reduce・approval・pass_output_from機構をそのまま利用）。

**対象コンポーネント**:

1. `src-tauri/src/workflow/builtin_facets/instructions/` — 21個のMDファイル新規作成
   - `plan-requirements-workflow.md` — 元ネタ: plan-requirements SKILL.md
   - `plan-behavior-workflow.md` — 元ネタ: plan-behavior SKILL.md
   - `plan-spec-workflow.md` — 元ネタ: plan-spec SKILL.md
   - `review-acceptance-spec.md` — 元ネタ: review/references/acceptance.md
   - `review-structure-spec.md` — 元ネタ: review/references/structure.md
   - `review-quality-spec.md` — 元ネタ: review/references/quality.md
   - `review-test-spec.md` — 元ネタ: review/references/test.md
   - `review-security-spec.md` — 元ネタ: review/references/security.md
   - `review-architecture-spec.md` — 元ネタ: review/references/architecture.md
   - `spec-fix-workflow.md` — 元ネタ: fix.md + plan系
   - `spec-approval-workflow.md` — 元ネタ: report.md
   - `implement-workflow.md` — 元ネタ: implement SKILL.md（品質チェック除外）
   - `quality-check-workflow.md` — 元ネタ: implement SKILL.md §5 品質ゲート
   - `review-acceptance-code.md` — 元ネタ: review/references/acceptance.md
   - `review-structure-code.md` — 元ネタ: review/references/structure.md
   - `review-quality-code.md` — 元ネタ: review/references/quality.md
   - `review-test-code.md` — 元ネタ: review/references/test.md
   - `review-security-code.md` — 元ネタ: review/references/security.md
   - `review-architecture-code.md` — 元ネタ: review/references/architecture.md
   - `implement-fix-workflow.md` — 元ネタ: fix.md + implement系
   - `implementation-approval-workflow.md` — 元ネタ: report.md

2. `src-tauri/src/workflow/builtin/spec-driven-development.yml` — ワークフローYAML新規作成
   - Issue #898の推奨YAML構成をベースに、品質チェックステップ（quality_check / fix_quality_check）を追加

3. `src-tauri/src/workflow/builtin.rs` — ビルトイン登録
   - `BUILTINS` 配列に `spec-driven-development.yml` のエントリを追加
   - `BUILTIN_FACETS` 配列に21個のinstruction facetエントリを追加（`include_str!` マクロ）

**影響するテスト**:
- Rust: `builtin.rs` の `init_creates_builtin_facets_in_empty_dir` テストに21個のassert追加
- Rust: `init_creates_builtins_in_empty_dir` テストに `spec-driven-development.yml` のassert追加
- Rust: ワークフローYAMLのパース検証（既存の `init_builtins::<Workflow>` で自動実行される）
