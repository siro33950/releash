## 要求

**種別**: 新機能
**ゴール**: ワークフローのYAMLスキーマを定義し、アプリデータディレクトリ（`~/.config/releash/workflows/`）で管理する基盤を構築する
**背景**: オーケストレーション機能（Milestone #49）のPhase 1。YAMLで定義したワークフローをSkills的に選択・実行し、計画→実装→レビュー→修正ループ→完了といったステップを自動遷移させるエンジンの基盤部分。親Issue: #691

**スコープ**:
- ワークフローYAMLスキーマ設計（steps, rules, modes, cycle_guard, prompts参照）
- プロンプトテンプレートYAMLスキーマ
- Rustパーサー + バリデーション
- `~/.config/releash/workflows/` への保存・読込
- ビルトインワークフローの同梱（quick-fix, plan-implement-review 等）
- Tauriコマンド: ワークフロー一覧取得、CRUD
- Setting画面にワークフロー一覧を表示
- 各ワークフローにZedで編集するボタンを設置

**制約**:
- 保存先はアプリデータ（リポジトリには配置しない）
- ステップモードは3種: `auto` / `approval` / `interactive`
- 1ステップ = 1 Agentセッション
- 品質チェックは専用システムではなくautoステップのプロンプトとして実現
- 編集はZed（外部エディタ）に委譲し、アプリ内にYAMLエディタは持たない

**検証方法**:
- YAMLファイルをパースし、ステップ一覧を正しく構造化できる
- 不正なYAML（未定義のモード、存在しないステップへの遷移等）でバリデーションエラーが返る
- ワークフローの保存・読込・一覧取得・削除がTauriコマンドで動作する
- ビルトインワークフローがアプリ初回起動時に配置される
- Setting画面にワークフロー一覧が表示される
- 「Zedで編集」ボタンでYAMLファイルが外部エディタで開かれる

## 振る舞い定義

```gherkin
Feature: ワークフロースキーマ & ストレージ
  YAMLで定義したワークフローをアプリデータディレクトリで管理し、
  パース・バリデーション・CRUD・ビルトイン同梱の基盤を提供する

  # --- パース & バリデーション ---

  Rule: 有効なYAMLはワークフロー構造体にパースされる
    Scenario: 全フィールドを持つワークフローをパースする
      Given steps, rules, modes, cycle_guard, promptsを含むYAMLファイルがある
      When ワークフローをパースする
      Then 各ステップのname, mode, prompt, rules, cycle_guardが構造化される

  Rule: 不正なYAMLはバリデーションエラーになる
    Scenario: 未定義のステップモードを指定する
      Given modeに "unknown" を指定したYAMLがある
      When ワークフローをパースする
      Then バリデーションエラーが返る

    Scenario: 存在しないステップへの遷移を指定する
      Given rulesのnextに定義されていないステップ名を指定したYAMLがある
      When ワークフローをパースする
      Then バリデーションエラーが返る

  # --- ストレージ CRUD ---

  Rule: ワークフローはアプリデータディレクトリに保存・読込される
    Scenario: ワークフローを保存する
      Given 有効なワークフロー定義がある
      When ワークフローを保存する
      Then ~/.config/releash/workflows/ にYAMLファイルが作成される

    Scenario: ワークフローを読み込む
      Given ~/.config/releash/workflows/ にYAMLファイルが存在する
      When ワークフローを読み込む
      Then ワークフロー構造体が返される

    Scenario: ワークフロー一覧を取得する
      Given ~/.config/releash/workflows/ に複数のYAMLファイルが存在する
      When ワークフロー一覧を取得する
      Then 全ワークフローの名前と説明のリストが返される

    Scenario: ワークフローを削除する
      Given ~/.config/releash/workflows/ にYAMLファイルが存在する
      When ワークフローを削除する
      Then YAMLファイルが削除される

  Rule: ビルトインワークフローは削除できない
    Scenario: ビルトインワークフローを削除しようとする
      Given ビルトインワークフロー（quick-fix等）が存在する
      When ビルトインワークフローを削除する
      Then エラーが返る

  # --- ビルトイン同梱 ---

  Rule: ビルトインワークフローはアプリ初回起動時に配置される
    Scenario: 初回起動でビルトインが配置される
      Given ~/.config/releash/workflows/ が存在しない
      When アプリが起動する
      Then ビルトインワークフロー（quick-fix, plan-implement-review等）が配置される

    Scenario: 既にワークフローが存在する場合はビルトインを上書きしない
      Given ~/.config/releash/workflows/ にユーザーが編集したビルトインワークフローがある
      When アプリが起動する
      Then ユーザーの編集内容が保持される

  # --- Setting画面 表示 ---

  Rule: Setting画面にワークフロー一覧が表示される
    Scenario: ワークフロー一覧を閲覧する
      Given ワークフローが3件保存されている
      When Setting画面のワークフローセクションを表示する
      Then 3件のワークフローが名前と説明付きで一覧表示される

  Rule: 各ワークフローにZedで編集するボタンが表示される
    Scenario: ワークフローをZedで編集する
      Given ワークフロー一覧が表示されている
      When ワークフローの「Zedで編集」ボタンを押す
      Then 該当のYAMLファイルがZedで開かれる
```

## 実装仕様

**対応方針**: 振る舞い定義を実現するために、Rustバックエンドに新モジュール `workflow` を追加し、YAMLスキーマ・パーサー・バリデーション・ストレージを実装する。フロントエンドはSetting画面に `workflows` セクションを追加し、一覧表示と外部エディタ連携を提供する。

**対象コンポーネント**:

### Rust（src-tauri/src/）
- `workflow/mod.rs`（新規）: モジュールルート
- `workflow/schema.rs`（新規）: YAMLスキーマの型定義（Workflow, Step, StepMode, Rule, CycleGuard）
- `workflow/prompt_schema.rs`（新規）: プロンプトテンプレートの型定義
- `workflow/storage.rs`（新規）: `~/.config/releash/workflows/` への保存・読込・一覧取得・削除
- `workflow/validation.rs`（新規）: パース後のバリデーション（未定義モード、遷移先不整合等）
- `workflow/builtin.rs`（新規）: ビルトインワークフローYAMLの埋め込み（`include_str!`）と初回配置ロジック
- `workflow/commands.rs`（新規）: Tauriコマンド（list_workflows, get_workflow, save_workflow, delete_workflow, open_workflow_in_editor）
- `lib.rs`: Tauriコマンド登録、起動時のビルトイン初期化呼び出し

### フロントエンド（src/）
- `components/panels/SettingsModal.tsx`: SettingsSection型に `"workflows"` 追加、SETTINGS_SECTIONS配列に項目追加、WorkflowsSection実装
- `hooks/useWorkflowConfig.ts`（新規）: ワークフロー一覧取得・削除・エディタ起動のフック

### ビルトインYAML（src-tauri/src/workflow/builtin/）
- `quick-fix.yml`（新規）: クイックフィックスワークフロー
- `plan-implement-review.yml`（新規）: 計画→実装→レビューループワークフロー

**技術選定**:
- `serde-saphyr`: YAML serde実装。serde_yaml / serde_yml が両方パブリックアーカイブのため、現在最も活発にメンテナンスされている代替クレートを採用（2026-04-25更新、62万DL）

**YAMLスキーマ（Rust型定義）**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub description: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub mode: StepMode,
    pub prompt: String,           // プロンプトテンプレート名
    #[serde(default)]
    pub rules: Vec<TransitionRule>,
    #[serde(default)]
    pub cycle_guard: Option<CycleGuard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepMode {
    Auto,
    Approval,
    Interactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionRule {
    pub r#match: String,          // パターン文字列
    pub next: String,             // 遷移先ステップ名
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleGuard {
    pub max_iterations: u32,
}
```

**ビルトイン判別方法**: YAMLに `builtin: true` フィールドを持たせ、削除時にチェック

**ストレージパターン**: 既存の `config.rs` の原子的書き込み（tmp→rename）パターンに準拠

**影響するテスト**:
- Rust単体テスト: schema.rs（パース正常系）、validation.rs（バリデーションエラー系）、storage.rs（CRUD操作）、builtin.rs（初回配置・上書きしない）
- フロントエンド: useWorkflowConfig.test.ts（Tauriコマンドモック）、SettingsModal.test.tsx（WorkflowsSection表示）
