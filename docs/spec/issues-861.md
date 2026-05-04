## 要求

**種別**: 新機能
**ゴール**: taktのファセット指向プロンプティングを採用し、プロンプトを独立ファセット（persona / policy / knowledge / instruction / output_contract）として管理し、ワークフローのステップごとにキーで参照・合成して適用する仕組みを構築する
**背景**: ワークフローエンジンの各ステップで、同一のpersonaを異なるpolicy/instructionと組み合わせる等、ファセット単位での再利用が可能な柔軟なプロンプト合成が必要
**制約**:
- 依存: #859 Workflow Schema & Storage（Phase 1完了後）
- #860 Workflow Engine Core と並行実装可
- 保存先: `~/.config/releash/workflows/` 配下にファセット種別ごとのディレクトリ（personas/, policies/, knowledge/, instructions/, output_contracts/）
- ビルトインファセット同梱（planner, coder, reviewer等）
- ファセットはMarkdownファイル（1ファセット = 1ファイル）
**影響範囲**: エンジン(#860)のステップ実行時にプロンプト合成を呼び出す。既存のAgentSDKセッション開始処理に統合

### ファセット構成（takt準拠）

| ファセット | 役割 | 配置 | 例 |
|------------|------|------|-----|
| Persona | エージェントの役割・専門知識 | system prompt | planner, coder, reviewer |
| Policy | 行動制約・品質基準・禁止事項 | user message末尾 | coding, security, review |
| Knowledge | ドメイン知識・参考資料 | user message | architecture, project-rules |
| Instruction | ステップ固有の手順・タスク指示 | user message | implement, review, plan |
| Output Contract | 出力形式の定義 | user message | review-report, plan-doc |

### ステップでの参照方法（ワークフローYAML内）

```yaml
steps:
  - name: implement
    persona: coder
    policy: coding
    instruction: implement
    knowledge: architecture
  - name: review
    persona: reviewer
    policy: review
    instruction: review
    knowledge: architecture
```

### 合成ルール
- Persona → Claude Code preset の append として system prompt に配置（既存の Claude Code 組み込み指示を保持）
- Knowledge → Instruction → Output Contract → Policy の順でuser messageに合成（Policyを最後に置くLLM特性に合わせた設計）
- テンプレート変数展開: `{{task}}`, `{{project_name}}` の自動注入（二重波括弧記法）
- 前ステップ出力の参照: `pass_previous_response` / `pass_output_from` による context block 追加（既存エンジン機能）

## 振る舞い定義

```gherkin
Feature: ファセット指向プロンプト合成
  ワークフローのステップごとにファセット（persona/policy/knowledge/instruction/output_contract）を
  キーで参照し、合成ルールに従って最終プロンプトを組み立てる

  Rule: ファセットの保存と読み込み
    Scenario: ファセットファイルを保存する
      Given ファセット種別 "persona" とキー "coder" のMarkdown内容が用意されている
      When ファセットを保存する
      Then ~/.config/releash/workflows/personas/coder.md にファイルが作成される

    Scenario: ファセットファイルを読み込む
      Given ~/.config/releash/workflows/personas/coder.md が存在する
      When ファセット種別 "persona" キー "coder" で読み込む
      Then ファイルのMarkdown内容が返される

    Scenario: 存在しないファセットを参照する
      Given ~/.config/releash/workflows/personas/unknown.md が存在しない
      When ファセット種別 "persona" キー "unknown" で読み込む
      Then ファセット未発見エラーが返される

  Rule: ビルトインファセットの初期化
    Scenario: 初回起動時にビルトインファセットが配置される
      Given ~/.config/releash/workflows/personas/ が空である
      When ビルトインファセットの初期化を実行する
      Then planner, coder, reviewer のpersonaファイルが配置される
      And 対応するpolicy, instruction ファイルも配置される

    Scenario: ユーザーが編集済みのビルトインファセットは上書きされない
      Given ~/.config/releash/workflows/personas/coder.md がユーザーにより編集済み
      When ビルトインファセットの初期化を実行する
      Then coder.md は上書きされず既存内容が保持される

  Rule: ステップへのファセット参照
    Scenario: ステップにファセットキーを指定する
      Given ワークフロー定義のステップに persona: "coder", policy: "coding", instruction: "implement" が指定されている
      When ワークフローを読み込む
      Then ステップのファセット参照が正しくパースされる

    Scenario: ステップのファセット参照が省略可能である
      Given ワークフロー定義のステップに persona のみ指定され policy/knowledge/instruction/output_contract が省略されている
      When ワークフローを読み込む
      Then 省略されたファセットはnullとしてパースされる

  Rule: プロンプト合成
    Scenario: 全ファセットが指定されたステップのプロンプトを合成する
      Given ステップに persona: "coder", policy: "coding", knowledge: "architecture", instruction: "implement", output_contract: "plan-doc" が指定されている
      And 各ファセットファイルが存在する
      When プロンプト合成を実行する
      Then system promptに persona の内容が配置される
      And user messageに knowledge → instruction → output_contract → policy の順で合成される

    Scenario: 一部ファセットが省略されたステップのプロンプトを合成する
      Given ステップに persona: "coder", instruction: "implement" のみ指定されている
      When プロンプト合成を実行する
      Then system promptに persona の内容が配置される
      And user messageに instruction の内容のみ配置される

    Scenario: テンプレート変数が展開される
      Given instruction ファセット内に "{{task}}" と "{{project_name}}" が含まれている
      And タスク内容 "バグを修正してください" が指定されている
      When プロンプト合成と変数展開を実行する
      Then "{{task}}" がタスク内容に置換される
      And "{{project_name}}" がworktreeパスから抽出されたプロジェクト名に置換される

    Scenario: 前ステップ出力がcontext blockとして追加される
      Given ステップに pass_previous_response: true が指定されている
      And 前ステップの出力が存在する
      When プロンプト合成後にステップ出力注入が実行される
      Then user message末尾に前ステップ出力が <step_output> ブロックとして追加される

  Rule: ファセット一覧の取得
    Scenario: 種別ごとのファセット一覧を取得する
      Given personas/ に "coder.md" と "reviewer.md" が存在する
      When persona 種別のファセット一覧を取得する
      Then "coder" と "reviewer" が一覧に含まれる
```

## 実装仕様

**対応方針**: ファセット指向プロンプト合成を実現するために、既存のワークフローモジュール（`src-tauri/src/workflow/`）にファセット管理機能を追加し、Bridge プロセスに system prompt 渡しを拡張する。

### 対象コンポーネント

| コンポーネント | 変更内容 |
|---------------|---------|
| `src-tauri/src/workflow/schema.rs` | Step 構造体にファセット参照フィールド追加（persona/policy/knowledge/instruction/output_contract） |
| `src-tauri/src/workflow/facet.rs` (新規) | ファセット種別enum、CRUD、合成ロジック |
| `src-tauri/src/workflow/storage.rs` | ファセットディレクトリ管理関数追加 |
| `src-tauri/src/workflow/builtin.rs` | ビルトインファセット（Markdown）の初期化 |
| `src-tauri/src/workflow/builtin_facets/` (新規) | ビルトインファセットMarkdownファイル群 |
| `src-tauri/src/workflow/engine.rs` | `start_step_session` でファセット合成を実行し、system prompt をセッション開始時に渡す |
| `src-tauri/src/workflow/commands.rs` | ファセットCRUD用Tauriコマンド追加 |
| `src-tauri/src/agent_sdk.rs` | `start_agent_session_internal` に `system_prompt: Option<String>` 追加 |
| `src-tauri/resources/claude-sdk-bridge.mjs` | `init` コマンドで `systemPrompt` を受け取り Agent SDK に渡す |
| `src-tauri/src/lib.rs` | `generate_handler!` にファセットCRUD用コマンドを登録。setup内で `init_builtin_facets(&workflow::storage::facets_base_dir())` を呼び出し追加（既存の `init_builtin_workflows` と同じ箇所） |
| `src-tauri/src/workflow/mod.rs` | `pub mod facet;` 追加 |
| `src/types/workflow.ts` | Step 型にファセット参照フィールド（persona/policy/knowledge/instruction/output_contract）を定義（旧StepPrompt型は廃止） |

### スキーマ変更

```rust
// schema.rs - Step 構造体（ファセット参照のみ、旧prompt方式は廃止済み）
pub struct Step {
    pub name: String,
    pub mode: StepMode,
    pub persona: Option<String>,              // ファセットキー
    pub policy: Option<String>,
    pub knowledge: Option<String>,
    pub instruction: Option<String>,
    pub output_contract: Option<String>,
    pub rules: Vec<TransitionRule>,
    pub cycle_guard: Option<CycleGuard>,
    pub pass_previous_response: Option<bool>,
    pub pass_output_from: Option<Vec<String>>,
    pub collect: Option<CollectConfig>,
}
```

**バリデーションルール** (R4):
- `collect` なしのステップはファセット参照（最低1つ）が必須。ない場合はエラー（`MissingFacet { step }`）
- `collect` ありのステップはファセット参照不要（セッション起動しないため）

### ファセットモジュール（`facet.rs`）

```rust
pub enum FacetKind {
    Persona,
    Policy,
    Knowledge,
    Instruction,
    OutputContract,
}

impl FacetKind {
    pub fn dir_name(&self) -> &str {
        match self {
            Self::Persona => "personas",
            Self::Policy => "policies",
            Self::Knowledge => "knowledge",
            Self::Instruction => "instructions",
            Self::OutputContract => "output_contracts",
        }
    }
}

pub struct ComposedPrompt {
    pub system_prompt: Option<String>,   // persona の内容
    pub user_message: String,            // knowledge + instruction + output_contract + policy
}

pub fn compose_facets(step: &Step, base_dir: &Path) -> Result<ComposedPrompt, FacetError>
pub fn load_facet(kind: FacetKind, key: &str, base_dir: &Path) -> Result<String, FacetError>
pub fn save_facet(kind: FacetKind, key: &str, content: &str, base_dir: &Path) -> Result<(), FacetError>
pub fn list_facets(kind: FacetKind, base_dir: &Path) -> Result<Vec<String>, FacetError>

/// ファセット専用キー検証（validate_name とは別関数として実装）
/// 既存 validate_name は先頭 - / _ を許可するが、ファセットキーはファイル名に直結するため
/// より厳格な制約を適用する
pub fn validate_facet_key(key: &str) -> Result<(), FacetError>
```

**キー検証ルール** (R2):
- `load_facet` / `save_facet` の入口で `validate_facet_key(key)` を呼び出す
- 許可パターン: `^[a-zA-Z0-9][a-zA-Z0-9_-]*$`（先頭は英数字必須、以降は英数字・ハイフン・アンダースコア）
- 既存 `validate_name`（先頭 `-` / `_` 許可）は再利用しない。ファセット専用の検証関数を `facet.rs` 内に実装する
- 違反時: `FacetError::InvalidKey { key }` を返す

### 合成順序

user message の合成順序（上から順に連結）:
1. Knowledge（ドメイン知識・背景情報）
2. Instruction（タスク手順）
3. Output Contract（出力形式）
4. Policy（行動制約 — LLM が最後に読んだ内容に強く影響される特性を活用）

各ファセット間は `\n\n` で区切る。

### Bridge 拡張 (R1)

Claude Code preset を維持しつつ persona を append する方式を採用する。
Agent SDK の `systemPrompt: { type: "preset", preset: "claude_code", append: "..." }` を使用し、
Claude Code の組み込み指示（ツール操作、レスポンススタイル等）を破壊しない。

既存の options フィールド（includePartialMessages, pathToClaudeCodeExecutable, stderr 等）は一切変更せず、`systemPrompt` フィールドのみを条件付きで追加する。

```javascript
// bridge.mjs - handleInit 内（既存フィールドを維持し systemPrompt のみ追加）
const options = {
    cwd: cmd.cwd,
    permissionMode,
    includePartialMessages: true,           // 既存: 維持
    settingSources: ["user", "project"],    // 既存: 維持
    pathToClaudeCodeExecutable: "claude",   // 既存: 維持
    stderr: (data) => { stderrChunks.push(data); },  // 既存: 維持
    // ↓ 新規追加: cmd.systemPrompt がある場合のみ
    ...(cmd.systemPrompt && {
        systemPrompt: {
            type: "preset",
            preset: "claude_code",
            append: cmd.systemPrompt,  // persona 内容を append
        },
    }),
};
```

```rust
// agent_sdk.rs - init コマンド
let mut init_cmd = serde_json::json!({
    "type": "init",
    "cwd": cwd,
    "permissionMode": initial_permission_mode,
    "sessionId": session_id,
});
if let Some(sp) = system_prompt {
    init_cmd["systemPrompt"] = serde_json::Value::String(sp);
}
```

Bridge 側で `cmd.systemPrompt` が文字列の場合に `{ type: "preset", preset: "claude_code", append: ... }` に変換する。

### エンジン変更（`engine.rs`）

`start_step_session` 内のフロー（ファセット方式のみ）:
1. ロック内で `step` 全体を clone する
2. ファセット参照が1つ以上あるか判定（`step.has_facet_refs()`）
3. **ファセット合成**:
   - `compose_facets(&step, facets_base_dir)` → `ComposedPrompt` を取得
   - `ComposedPrompt.system_prompt` を `start_agent_session_internal` に渡す
   - `ComposedPrompt.user_message` にテンプレート変数展開（`{{task}}`, `{{project_name}}`）を適用
   - `inject_step_outputs()` を適用 → 最終 user message
4. **ファセット参照なしの場合**: バリデーションで弾かれているはずだが、防御的に `WorkflowEngineError::InvalidWorkflow` を返す

### テンプレート変数

**記法**: `{{variable_name}}`（二重波括弧。既存 prompt template の `{{project_name}}` と同じ記法）

合成後の user message 内で自動展開:

| 変数 | 内容 | 展開タイミング |
|------|------|--------------|
| `{{task}}` | ワークフロー実行時のタスク内容（未指定時は空文字列に展開） | compose_facets の後、engine 側で展開 |
| `{{project_name}}` | worktree パスから自動抽出 | compose_facets の後、engine 側で展開 |

前ステップ出力の参照は `pass_previous_response` / `pass_output_from` による context block 追加（`inject_step_outputs` 既存機能）で行う。テンプレート変数としては扱わない。

**展開の責務配置**:
- `compose_facets(step, base_dir)` は純粋な合成のみ（ファセットファイル読み込み → 順序通り連結）。変数展開はしない
- engine.rs 側に `render_facet_variables(content: &str, task: Option<&str>, worktree_path: &str) -> String` を配置
- `start_step_session` 内で `compose_facets` の結果に対して `render_facet_variables` を呼び出す
- テスト: 変数展開のテストは `engine.rs` のテストモジュールに配置（`facet.rs` ではない）

**`{task}` の取得元** (R5):
- `start_workflow` コマンドに `task: Option<String>` パラメータを追加する
- task が指定された場合: そのテキストを `{task}` に展開
- task が未指定の場合: `{{task}}` を空文字列に展開する
- `WorkflowExecution` 構造体に `task: Option<String>` フィールドを追加し、実行中のステップで参照可能にする
- **UI側の呼び出し変更**: `WorkflowPanel.tsx` の `invoke("start_workflow", ...)` に `task` パラメータを追加。ワークフロー起動UIにタスク入力欄（テキストエリア）を設け、ユーザーが入力した内容を渡す。入力が空の場合は `task: null` として送信する

### ビルトインファセット (R6)

`src-tauri/src/workflow/builtin_facets/` に配置:

| パス | ファセット種別 | キー |
|------|--------------|------|
| `personas/planner.md` | Persona | planner |
| `personas/coder.md` | Persona | coder |
| `personas/reviewer.md` | Persona | reviewer |
| `policies/coding.md` | Policy | coding |
| `policies/review.md` | Policy | review |
| `instructions/plan.md` | Instruction | plan |
| `instructions/implement.md` | Instruction | implement |
| `instructions/review.md` | Instruction | review |
| `instructions/fix.md` | Instruction | fix |
| `instructions/verify.md` | Instruction | verify |
| `instructions/report.md` | Instruction | report |
| `instructions/test-step.md` | Instruction | test-step |
| `knowledge/test-context.md` | Knowledge | test-context |
| `output_contracts/test-report.md` | Output Contract | test-report |

### ビルトインワークフロー: trace-test

ファセット合成・テンプレート変数展開・ルールベース遷移・cycle guard の動作確認用ビルトインワークフロー。
`builtin/trace-test.yml` として同梱する。

```yaml
name: trace-test
description: ワークフロー動作テスト用（全ファセット確認 + ループ）
builtin: true

steps:
  - name: plan
    mode: interactive
    persona: planner
    instruction: test-step
    knowledge: test-context
    output_contract: test-report

  - name: execute
    mode: auto
    persona: coder
    instruction: test-step
    policy: coding
    knowledge: test-context
    output_contract: test-report
    pass_previous_response: true
    rules:
      - match: NEEDS_FIX
        next: execute
      - match: DONE
        next: confirm
    cycle_guard:
      max_iterations: 3

  - name: confirm
    mode: approval
    persona: reviewer
    instruction: test-step
    policy: review
    knowledge: test-context
    output_contract: test-report
    pass_previous_response: true
```

`test-step` instruction ファセットは全ステップ共通のテスト用指示で、全5種ファセット（persona / policy / knowledge / instruction / output_contract）の合成と DONE / NEEDS_FIX のループ機構を検証する。`test-context` knowledge ファセットはファセット合成の事実を提供し、`test-report` output contract は観察結果の出力形式を定義する。

**ビルトインに含めないファセット**（ユーザー作成例）:
- `knowledge/architecture.md` — プロジェクト固有のドメイン知識のためビルトイン不適
- `output_contracts/plan-doc.md` — プロジェクト固有の出力形式のためビルトイン不適

要求セクションのステップ例（knowledge: architecture, output_contract: plan-doc）はユーザーが独自に作成するファセットの例である。

初期化: 固定マニフェスト配列を走査し、対象パスにファイルが存在しない場合のみ書き出す。

```rust
struct BuiltinFacetEntry {
    kind: FacetKind,
    key: &'static str,
    content: &'static str,
}

const BUILTIN_FACETS: &[BuiltinFacetEntry] = &[
    BuiltinFacetEntry { kind: FacetKind::Persona, key: "planner", content: include_str!("builtin_facets/personas/planner.md") },
    BuiltinFacetEntry { kind: FacetKind::Persona, key: "coder", content: include_str!("builtin_facets/personas/coder.md") },
    // ... 他エントリ
];

pub fn init_builtin_facets(base_dir: &Path) -> Result<(), BuiltinInitError> {
    for entry in BUILTIN_FACETS {
        let dir = base_dir.join(entry.kind.dir_name());
        storage::ensure_dir(&dir)?;
        let file_path = dir.join(format!("{}.md", entry.key));
        if file_path.exists() {
            continue;  // ユーザーカスタマイズ保護
        }
        std::fs::write(&file_path, entry.content)?;
    }
    Ok(())
}
```

### 影響するテスト

| テスト種別 | 内容 |
|-----------|------|
| Rust単体テスト (`facet.rs`) | CRUD、合成ロジック（全指定/一部省略）、キー検証、エラーケース |
| Rust単体テスト (`schema.rs`) | ファセットフィールド付き Step のデシリアライズ |
| Rust単体テスト (`builtin.rs`) | ビルトインファセット初期化、上書き保護 |
| Rust単体テスト (`engine.rs`) | ファセット方式での start_step_session フロー（facet-only step の分岐確認）、`render_facet_variables` のテンプレート変数展開 |
| Rust単体テスト (`validation.rs`) | collectなしstepのファセット必須チェック |
| Rust単体テスト (`commands.rs`) | start_workflow の task パラメータ受け渡し |
| Bridge テスト (`bridge.mjs`) | `cmd.systemPrompt` が文字列で渡された場合に `{ type: "preset", preset: "claude_code", append: ... }` に変換されること |
| コマンド登録確認 (`lib.rs`) | ファセットCRUDコマンドが `generate_handler!` に登録されていること |
| フロントエンド型 (`workflow.ts`) | Step 型にファセットフィールドが定義され、Optional であること |
